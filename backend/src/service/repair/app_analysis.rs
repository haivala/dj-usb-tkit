//! Analysis bundles the repairs draw, and the one fix for bundles this app
//! wrote (`update_app_analysis_files`). Every bundle drawn here comes from
//! the library's analysis pipeline (`analyze_audio_core`,
//! `write_track_bundle`), never from a drawing of the repairs' own.

use super::*;
use crate::i18n::tr;
use crate::service::analysis::{
    CoreAnalysis, CoreRequest, FileAnalysis, TrackBundle, analysis_settings, write_track_bundle,
};
use crate::service::anlz::{pwv3_stretched_from_bins, waveform_chunks, with_waveform_chunks};
use std::path::PathBuf;

/// The bins the USB repairs drew a waveform with before 0.6.7, stretched to
/// the detail length: what `update_app_analysis_files` recognizes their
/// bundles by. A record of what those versions wrote, so it never follows
/// any size the app draws or shows waveforms at now.
pub(super) const PRE_0_6_7_REPAIR_WAVEFORM_BINS: usize = 2400;

/// A USB file for [`BackendService::analyze_stick_files`].
pub(super) struct StickAudio {
    pub(super) audio: PathBuf,
    /// The databases' BPM, kept; detected when `None`.
    pub(super) kept_bpm: Option<f64>,
    pub(super) detect_key: bool,
}

/// Why a USB file's bundle wasn't drawn.
#[derive(Debug)]
pub(super) enum DrawError {
    /// The audio doesn't decode, or gives no waveform.
    Unreadable,
    /// Its length is unknown, so there is no time axis to draw on.
    LengthUnknown,
    /// Analysis was cancelled before the file's turn came.
    Cancelled,
    Write(BackendError),
}

/// A USB file, analysed as a library track is: its waveform at detail
/// resolution over the whole track and its length, the databases' BPM kept
/// (else one detected) with its first beat placed, and its key when asked
/// for. Write each with [`write_stick_bundle`] one at a time, since a new
/// bundle's name depends on which ones exist.
#[derive(Debug, Clone)]
pub(super) struct StickAnalysis {
    pub(super) core: CoreAnalysis,
    pub(super) duration_ms: u64,
}

impl BackendService {
    /// Analyses USB files in the one analysis queue
    /// (`analyze_audio_files`): after whatever is waiting, with library
    /// analysis's engine and BPM range, paused and cancelled with it.
    /// Results in the files' order.
    pub(super) fn analyze_stick_files(
        &self,
        files: Vec<StickAudio>,
    ) -> BackendResult<Vec<Result<StickAnalysis, DrawError>>> {
        let conn = self.db.connect()?;
        let (engine, bpm_min, bpm_max) = analysis_settings(&conn);
        drop(conn);
        let requests = files
            .into_iter()
            .map(|file| {
                let request = CoreRequest {
                    kept_bpm: file.kept_bpm,
                    kept_first_beat_ms: None,
                    existing_first_beat_ms: None,
                    detect_key: file.detect_key,
                    // A file whose BPM can't be found (silence) still gets
                    // its waveform, with no beat grid.
                    detection_required: false,
                    bpm_min,
                    bpm_max,
                    engine,
                };
                (file.audio, request)
            })
            .collect();
        Ok(self
            .analyze_audio_files(requests)?
            .into_iter()
            .map(|result| match result {
                FileAnalysis::Done(core) => {
                    let Some(duration_ms) = core.duration_ms else {
                        return Err(DrawError::LengthUnknown);
                    };
                    if core.waveform.peaks.is_empty() {
                        return Err(DrawError::Unreadable);
                    }
                    Ok(StickAnalysis {
                        core: *core,
                        duration_ms,
                    })
                }
                FileAnalysis::Failed(err) => {
                    logging::debug(
                        "usb-repair",
                        format!("audio not analyzed: {}", err.english()),
                    );
                    Err(DrawError::Unreadable)
                }
                FileAnalysis::Cancelled => Err(DrawError::Cancelled),
            })
            .collect())
    }
}

/// Writes a USB file's new bundle from its [`StickAnalysis`] with the
/// library's bundle writer, the `PPTH` naming `track_path`, and for a FLAC
/// the `PVB2` the repairs have always added to the bundles they draw. No
/// cues.
pub(super) fn write_stick_bundle(
    analysis: &StickAnalysis,
    paths: &AnlzBundlePaths,
    track_path: &str,
    audio: &Path,
) -> Result<(), DrawError> {
    write_track_bundle(
        &TrackBundle {
            waveform: &analysis.core.waveform,
            track_path,
            bpm: analysis.core.bpm,
            duration_ms: analysis.duration_ms,
            first_beat_ms: analysis.core.first_beat_ms,
            cues: &[],
        },
        paths,
        audio,
    )
    .map_err(DrawError::Write)?;
    add_flac_seek_index(&paths.dat_path, audio);
    Ok(())
}

/// A FLAC's `PVB2` in the `.EXT` of a bundle a repair just drew, so the FLAC
/// seek-data fix doesn't find it on the next pass. A file whose index can't
/// be built keeps none; that fix lists it as unsupported.
fn add_flac_seek_index(dat: &Path, audio: &Path) {
    if SeekFormat::of(&audio.to_string_lossy()) != Some(SeekFormat::Flac) {
        return;
    }
    let ext = dat.with_extension("EXT");
    if let (Ok(Some(index)), Ok(bytes)) = (seek_index_for_audio(audio), std::fs::read(&ext))
        && let Some(patched) = with_seek_index(&bytes, &index)
    {
        let _ = atomic_write_bytes(&ext, &patched);
    }
}

/// Put `track_path` into a bundle's `PPTH` (`.DAT`, `.EXT`, `.2EX`).
/// Returns whether a file changed.
pub(super) fn stamp_bundle_ppth(dat: &Path, track_path: &str) -> BackendResult<bool> {
    let mut changed = false;
    for file in bundle_files(dat) {
        let Ok(data) = std::fs::read(&file) else {
            continue;
        };
        let patched = ensure_ppth_chunk(&data, track_path);
        if patched != data {
            atomic_write_bytes(&file, &patched)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn bundle_files(dat: &Path) -> [PathBuf; 3] {
    [
        dat.to_path_buf(),
        dat.with_extension("EXT"),
        dat.with_extension("2EX"),
    ]
}

// ---------------------------------------------------------------------------
// update_app_analysis_files
// ---------------------------------------------------------------------------

/// Why a bundle's waveform is replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WaveformFault {
    /// Drawn by a USB repair before 0.6.7: 2,400 bins stretched to the
    /// detail length ([`pwv3_stretched_from_bins`]).
    RepairDrawn,
    /// Exported before 0.6.7, longer than analysis then decoded: its first
    /// part stretched over its whole length. Found by comparing it with the
    /// library's.
    Stretched,
}

/// Where a bundle's new waveform comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WaveformSource {
    /// The library's bundle for the track (or its transcode's).
    Library(PathBuf),
    /// The library track has no waveform yet: library analysis draws it
    /// first, then it is copied.
    AnalyzeLibrary(String),
    /// The transcode's waveform bundle is gone: drawn from its file first.
    Transcode { dir: PathBuf, file: PathBuf },
    /// Not in the library: drawn from the USB's audio.
    Draw,
}

/// A track whose analysis bundle this app wrote and that needs updating,
/// with everything it needs.
#[derive(Debug, Clone, Default)]
pub(super) struct AppBundleUpdate {
    pub(super) track_id: u32,
    pub(super) track_path: String,
    /// Its bundle's `.DAT` now.
    pub(super) dat: PathBuf,
    /// Shares another track's bundle: gets its own, drawn from its audio.
    pub(super) own_bundle: bool,
    pub(super) waveform: Option<(WaveformFault, WaveformSource)>,
    /// Its beat grid is in the pre-0.3.7 format.
    pub(super) grid: bool,
    /// Its bundle doesn't name it in `PPTH`.
    pub(super) track_path_missing: bool,
    /// An MP3 whose `.DAT` has an empty seek index, and where to write it.
    pub(super) mp3_seek: Option<MissingSeekData>,
}

impl AppBundleUpdate {
    /// The reasons, for the Event Log, each in the renderer's language.
    pub(super) fn reasons(&self, r: fn(&crate::i18n::Text) -> String) -> String {
        let mut out = Vec::new();
        if self.own_bundle {
            out.push(r(&tr::repair_app_analysis_reason_shared()));
        }
        match self.waveform.as_ref().map(|(fault, _)| fault) {
            Some(WaveformFault::RepairDrawn) => {
                out.push(r(&tr::repair_app_analysis_reason_repair_drawn()));
            }
            Some(WaveformFault::Stretched) => {
                out.push(r(&tr::repair_app_analysis_reason_stretched()));
            }
            None => {}
        }
        if self.grid {
            out.push(r(&tr::repair_app_analysis_reason_grid()));
        }
        if self.track_path_missing {
            out.push(r(&tr::repair_app_analysis_reason_track_path()));
        }
        if self.mp3_seek.is_some() {
            out.push(r(&tr::repair_app_analysis_reason_mp3_seek()));
        }
        out.join(", ")
    }
}

/// A library track, as far as matching a USB track to it needs.
#[derive(Debug, Clone)]
struct LibraryTrack {
    id: String,
    file_path: String,
    duration_ms: Option<i64>,
    waveform_peaks_path: Option<String>,
}

/// The library, indexed the exact ways a USB track is matched to it.
#[derive(Default)]
struct LibraryIndex {
    tracks: Vec<LibraryTrack>,
    by_id: HashMap<String, usize>,
    by_path: HashMap<String, usize>,
    by_fingerprint: HashMap<ContentFingerprint, Vec<usize>>,
    /// masterContentId -> `track_export_identity` key (`<id>` or
    /// `<id>:<format>`).
    by_export_identity: HashMap<u32, String>,
}

fn library_path_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

impl LibraryIndex {
    fn load(conn: &rusqlite::Connection) -> BackendResult<Self> {
        let mut index = Self::default();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_size_bytes, title, artist, duration_ms, \
             waveform_peaks_path FROM tracks",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                LibraryTrack {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    duration_ms: row.get(5)?,
                    waveform_peaks_path: row.get(6)?,
                },
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (track, size, title, artist) = row?;
            let at = index.tracks.len();
            index.by_id.insert(track.id.clone(), at);
            index.by_path.insert(library_path_key(&track.file_path), at);
            if let Some(fingerprint) = content_fingerprint_key(size, &title, &artist) {
                index
                    .by_fingerprint
                    .entry(fingerprint)
                    .or_default()
                    .push(at);
            }
            index.tracks.push(track);
        }
        BackendService::ensure_track_export_identity_schema(conn)?;
        let mut stmt =
            conn.prepare("SELECT track_id, master_content_id FROM track_export_identity")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (key, id) = row?;
            if let Ok(id) = u32::try_from(id) {
                index.by_export_identity.insert(id, key);
            }
        }
        Ok(index)
    }

    /// The library track a USB track is, exactly, and the transcode format
    /// when the USB file is its transcode: the track export wrote that file
    /// from (the row's app identity), else the library track at the same
    /// file, else the one with the same content (size, title, artist) and
    /// length. Anything looser counts as not in the library: a wrong match
    /// would write another version's waveform for good.
    fn find(
        &self,
        parsed: &ParsedPdb,
        row: &crate::pdb_reader::PdbTrackRow,
        app_master_db_id: Option<u32>,
        audio: Option<&Path>,
    ) -> Option<(&LibraryTrack, Option<crate::audio_encode::TranscodeFormat>)> {
        if app_master_db_id.is_some()
            && row.master_db_id == app_master_db_id
            && let Some(key) = row
                .master_content_id
                .and_then(|id| self.by_export_identity.get(&id))
        {
            let (id, format) = match key.split_once(':') {
                Some((id, format)) => (
                    id,
                    Some(crate::audio_encode::TranscodeFormat::parse(format)?),
                ),
                None => (key.as_str(), None),
            };
            return self.by_id.get(id).map(|&at| (&self.tracks[at], format));
        }
        if let Some(&at) = audio.and_then(|audio| {
            self.by_path
                .get(&library_path_key(&audio.to_string_lossy()))
        }) {
            return Some((&self.tracks[at], None));
        }
        let fingerprint = pdb_track_fingerprint(parsed, row)?;
        let seconds = i64::from(row.duration_seconds.filter(|s| *s > 0)?);
        let mut same = self
            .by_fingerprint
            .get(&fingerprint)?
            .iter()
            .map(|&at| &self.tracks[at])
            .filter(|track| {
                track
                    .duration_ms
                    .is_some_and(|ms| (ms / 1000 - seconds).abs() <= 1)
            });
        let track = same.next()?;
        same.next().is_none().then_some((track, None))
    }
}

/// Where the library keeps the waveform for `track` (or its transcode).
/// `None` when it can't give one: the track's file is gone, or its
/// transcode is.
fn library_waveform_source(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    track: &LibraryTrack,
    transcode: Option<crate::audio_encode::TranscodeFormat>,
) -> Option<WaveformSource> {
    use crate::service::transcode::{transcode_bundle_dir, transcode_waveform_bundle};
    if let Some(format) = transcode {
        let file: String = conn
            .query_row(
                "SELECT file_path FROM track_transcodes WHERE track_id = ?1 AND format = ?2",
                params![track.id, format.as_str()],
                |row| row.get(0),
            )
            .ok()?;
        let file = PathBuf::from(file);
        let dir = transcode_bundle_dir(data_dir, &file);
        let dat = transcode_waveform_bundle(&dir).dat_path;
        return if dat.is_file() {
            Some(WaveformSource::Library(dat))
        } else if file.is_file() {
            Some(WaveformSource::Transcode { dir, file })
        } else {
            None
        };
    }
    if !Path::new(&track.file_path).is_file() {
        return None;
    }
    Some(
        match track
            .waveform_peaks_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .filter(|p| !crate::service::usb::stick_files::local_analysis_bundle_unusable(p))
        {
            Some(dat) => WaveformSource::Library(PathBuf::from(dat)),
            None => WaveformSource::AnalyzeLibrary(track.id.clone()),
        },
    )
}

/// Whether the waveforms of two bundles' `.EXT`s differ (the `.EXT` holds
/// the detail waveforms a stretched one shows in). Unreadable counts as
/// different.
fn waveforms_differ(stick_dat: &Path, library_dat: &Path) -> bool {
    let read = |dat: &Path| std::fs::read(dat.with_extension("EXT")).ok();
    match (read(stick_dat), read(library_dat)) {
        (Some(stick), Some(library)) => waveform_chunks(&stick) != waveform_chunks(&library),
        _ => true,
    }
}

/// Whether a USB track is at least as long as analysis before 0.6.7
/// decoded: longer ones it drew stretched.
fn longer_than_old_decode_cap(row: &crate::pdb_reader::PdbTrackRow) -> bool {
    let rate = u64::from(row.sample_rate_hz.filter(|r| *r > 0).unwrap_or(44_100));
    row.duration_seconds.is_some_and(|s| {
        u64::from(s) * rate >= crate::service::analysis::BPM_KEY_MAX_SAMPLES as u64
    })
}

/// What [`BackendService::detect_app_bundle_updates`] reads besides the PDB:
/// the other detectors' findings on the same stick.
pub(super) struct AppBundleFindings<'a> {
    /// Whose each bundle is (`scan_analysis_ownership`, reading `PPTH`).
    pub(super) ownership: &'a AnalysisOwnership,
    /// `.DAT`s whose beat grid is in the pre-0.3.7 format
    /// (`detect_outdated_beat_grids`).
    pub(super) outdated_grids: &'a [PathBuf],
    /// MP3s without seek data the MP3 seek fix could add (not remembered
    /// as skipped).
    pub(super) mp3_missing_seek: &'a [MissingSeekData],
}

impl BackendService {
    /// The tracks whose app-written bundle needs updating
    /// (`update_app_analysis_files`), by PDB track id. Reads only analysis
    /// files and databases, never audio. rekordbox's own bundles never match:
    /// each check is one only this app's bundles can fail.
    pub(super) fn detect_app_bundle_updates(
        &self,
        usb_root: &Path,
        parsed: &ParsedPdb,
        app_master_db_id: Option<u32>,
        findings: &AppBundleFindings<'_>,
    ) -> BackendResult<Vec<AppBundleUpdate>> {
        let ownership = findings.ownership;
        let mut updates = std::collections::BTreeMap::<u32, AppBundleUpdate>::new();
        let rows = parsed
            .tracks
            .iter()
            .map(|t| (t.id, t))
            .collect::<HashMap<_, _>>();
        let non_owners = ownership.non_owner_track_ids();
        fn get<'a>(
            updates: &'a mut std::collections::BTreeMap<u32, AppBundleUpdate>,
            rows: &HashMap<u32, &crate::pdb_reader::PdbTrackRow>,
            id: u32,
            dat: &Path,
        ) -> Option<&'a mut AppBundleUpdate> {
            let row = rows.get(&id)?;
            Some(updates.entry(id).or_insert_with(|| AppBundleUpdate {
                track_id: id,
                track_path: row.track_file_path.clone(),
                dat: dat.to_path_buf(),
                ..AppBundleUpdate::default()
            }))
        }
        for bundle in &ownership.shared {
            for &id in &bundle.track_ids {
                if let Some(update) = get(&mut updates, &rows, id, &bundle.dat) {
                    if Some(id) == bundle.owner {
                        update.track_path_missing = true;
                    } else {
                        update.own_bundle = true;
                    }
                }
            }
        }
        for bundle in &ownership.unmarked {
            if let Some(update) = get(&mut updates, &rows, bundle.track_id, &bundle.dat) {
                update.track_path_missing = true;
            }
        }

        // Each other check is about a track's own bundle: a track that gets
        // a new one is left out of them.
        let resolve = |row: &crate::pdb_reader::PdbTrackRow| {
            crate::service::usb::root::resolve_usb_side_path(usb_root, &row.anlz_path)
        };
        let outdated = findings
            .outdated_grids
            .iter()
            .map(|dat| dat.to_string_lossy().to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let mp3_missing = findings
            .mp3_missing_seek
            .iter()
            .map(|m| (audio_path_key(&m.track_path), m))
            .collect::<HashMap<_, _>>();
        let mut library: Option<LibraryIndex> = None;
        let library_conn = self.db.connect()?;
        let data_dir = self.db.data_dir();
        for row in &parsed.tracks {
            if non_owners.contains(&row.id) {
                continue;
            }
            let Some(dat) = resolve(row) else {
                continue;
            };
            if outdated.contains(&dat.to_string_lossy().to_ascii_lowercase())
                && let Some(update) = get(&mut updates, &rows, row.id, &dat)
            {
                update.grid = true;
            }
            if let Some(target) = mp3_missing.get(&audio_path_key(&row.track_file_path))
                && let Some(update) = get(&mut updates, &rows, row.id, &dat)
            {
                update.mp3_seek = Some((*target).clone());
            }

            let repair_drawn = read_anlz_chunks(&dat.with_extension("EXT"), &[*b"PWV3"])
                .and_then(|ext| pwv3_stretched_from_bins(&ext, PRE_0_6_7_REPAIR_WAVEFORM_BINS))
                == Some(true);
            let exported_long = app_master_db_id.is_some()
                && row.master_db_id == app_master_db_id
                && longer_than_old_decode_cap(row);
            if !repair_drawn && !exported_long {
                continue;
            }
            if library.is_none() {
                library = Some(LibraryIndex::load(&library_conn)?);
            }
            let audio =
                crate::service::usb::root::resolve_usb_side_path(usb_root, &row.track_file_path);
            let source = library
                .as_ref()
                .and_then(|index| index.find(parsed, row, app_master_db_id, audio.as_deref()))
                .and_then(|(track, transcode)| {
                    library_waveform_source(&library_conn, &data_dir, track, transcode)
                });
            let waveform = if repair_drawn {
                Some((
                    WaveformFault::RepairDrawn,
                    source.unwrap_or(WaveformSource::Draw),
                ))
            } else {
                // A stretched waveform can't be told from the bundle alone:
                // only a library waveform to compare with finds it.
                match source {
                    Some(WaveformSource::Library(library_dat)) => {
                        waveforms_differ(&dat, &library_dat).then_some((
                            WaveformFault::Stretched,
                            WaveformSource::Library(library_dat),
                        ))
                    }
                    Some(other) => Some((WaveformFault::Stretched, other)),
                    None => None,
                }
            };
            if let Some(waveform) = waveform
                && let Some(update) = get(&mut updates, &rows, row.id, &dat)
            {
                update.waveform = Some(waveform);
            }
        }
        Ok(updates.into_values().collect())
    }
}

/// What [`BackendService::apply_update_app_analysis_files`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct AppBundleOutcome {
    /// Tracks whose bundle was updated in place.
    pub(super) updated: usize,
    /// Tracks given a bundle of their own.
    pub(super) separated: usize,
    /// Tracks whose MP3 seek index couldn't be built, each named in the
    /// Event Log, and nothing else changed.
    pub(super) skipped: usize,
    /// Tracks left as they were, each named in the Event Log.
    pub(super) failed: usize,
}

/// The waveform files a bundle's waveforms are copied from, by extension.
struct WaveformFiles {
    dat: Option<Vec<u8>>,
    ext: Option<Vec<u8>>,
    twoex: Option<Vec<u8>>,
}

impl WaveformFiles {
    fn read(dat: &Path) -> Self {
        Self {
            dat: std::fs::read(dat).ok(),
            ext: std::fs::read(dat.with_extension("EXT")).ok(),
            twoex: std::fs::read(dat.with_extension("2EX")).ok(),
        }
    }

    /// A drawn waveform, laid out as the bundle writer lays it out.
    fn drawn(analysis: &StickAnalysis) -> Self {
        use crate::service::anlz::{build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file};
        let waveform = &analysis.core.waveform;
        let duration_ms = analysis.duration_ms;
        Self {
            dat: Some(build_anlz_dat_file(
                waveform,
                "",
                None,
                duration_ms,
                None,
                &[],
            )),
            ext: Some(build_anlz_ext_file(
                waveform,
                "",
                None,
                duration_ms,
                None,
                &[],
            )),
            twoex: Some(build_anlz_2ex_file(waveform, "", duration_ms)),
        }
    }

    /// Puts these waveforms into the bundle at `dat`, keeping everything
    /// else in it. A file of the bundle that is missing or doesn't walk
    /// cleanly is left as it is.
    fn copy_into(&self, dat: &Path) -> BackendResult<bool> {
        let mut changed = false;
        for (file, source) in bundle_files(dat)
            .into_iter()
            .zip([&self.dat, &self.ext, &self.twoex])
        {
            let (Ok(bytes), Some(source)) = (std::fs::read(&file), source) else {
                continue;
            };
            if let Some(patched) = with_waveform_chunks(&bytes, source)
                && patched != bytes
            {
                atomic_write_bytes(&file, &patched)?;
                changed = true;
            }
        }
        Ok(changed)
    }
}

/// Why one track's bundle wasn't updated, for its Event Log line.
enum UpdateFailure {
    Unreadable,
    LibraryAnalysis,
    Cancelled,
    Write(BackendError),
}

impl From<DrawError> for UpdateFailure {
    fn from(err: DrawError) -> Self {
        match err {
            DrawError::Cancelled => Self::Cancelled,
            DrawError::Write(err) => Self::Write(err),
            DrawError::Unreadable | DrawError::LengthUnknown => Self::Unreadable,
        }
    }
}

impl BackendService {
    /// Updates every bundle [`Self::detect_app_bundle_updates`] finds on the
    /// staged PDB as this pass left it (`update_app_analysis_files`). Only
    /// the waveform's values come from elsewhere: the library's bundle (drawn
    /// first by library analysis when it has none yet), else the stick's
    /// audio through the analysis pipeline. BPM, beat grid positions, cues
    /// and what players wrote (`PVB2`) are kept. Each bundle is then in the
    /// current format: the beat grid in rekordbox's layout, a `PPTH` naming
    /// the track, its own bundle when it shared one (its PDB row and eDB
    /// content row point there), and the MP3 seek index.
    pub(super) fn apply_update_app_analysis_files(
        &self,
        usb_root: &Path,
        app_master_db_id: Option<u32>,
        edb_conn: Option<&mut rusqlite::Connection>,
        seek_skips: &SeekSkipMemory,
        warnings: &mut Vec<WarningEntry>,
        on_event: &mut dyn FnMut(SeekApplyEvent),
    ) -> BackendResult<AppBundleOutcome> {
        let pdb_path = staging::required_pdb(usb_root)?;
        let mut pdb_bytes = std::fs::read(&pdb_path)?;
        let page_size = pdb_page_size(&pdb_bytes)?;
        let parsed = crate::pdb_reader::parse_pdb_bytes(&pdb_bytes)?;
        let outdated_grids =
            crate::service::diagnostics::detect_outdated_beat_grids(usb_root, &parsed)
                .unwrap_or_default()
                .into_iter()
                .map(|grid| grid.dat)
                .collect::<Vec<_>>();
        let (mp3_missing_seek, _) = partition_remembered_seek_skips(
            detect_missing_seek_data(usb_root, &parsed, SeekFormat::Mp3),
            seek_skips,
        );
        let mut updates = self.detect_app_bundle_updates(
            usb_root,
            &parsed,
            app_master_db_id,
            &AppBundleFindings {
                ownership: &scan_analysis_ownership(usb_root, &parsed, app_master_db_id, true),
                outdated_grids: &outdated_grids,
                mp3_missing_seek: &mp3_missing_seek,
            },
        )?;
        let total = updates.len();
        let row_by_id = parsed
            .tracks
            .iter()
            .map(|t| (t.id, t))
            .collect::<HashMap<_, _>>();
        let mut failures = HashMap::<u32, UpdateFailure>::new();

        // Library tracks without a waveform yet: library analysis draws it,
        // through its coordinator, then it is copied like any other.
        let to_analyze = updates
            .iter()
            .filter_map(|u| match &u.waveform {
                Some((_, WaveformSource::AnalyzeLibrary(id))) => Some(id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        if !to_analyze.is_empty() {
            let analyzed = self.analyze_new_tracks(crate::models::AnalyzeNewTracksRequest {
                track_ids: to_analyze.into_iter().collect(),
                ..Default::default()
            });
            warnings.extend(analyzed.map(|data| data.warnings).unwrap_or_default());
            let conn = self.db.connect()?;
            for update in &mut updates {
                let Some((fault, WaveformSource::AnalyzeLibrary(id))) = &update.waveform else {
                    continue;
                };
                let path: Option<String> = conn
                    .query_row(
                        "SELECT waveform_peaks_path FROM tracks WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok()
                    .flatten();
                match path.filter(|p| {
                    !p.trim().is_empty()
                        && !crate::service::usb::stick_files::local_analysis_bundle_unusable(p)
                }) {
                    Some(path) => {
                        update.waveform =
                            Some((*fault, WaveformSource::Library(PathBuf::from(path))));
                    }
                    None => {
                        failures.insert(update.track_id, UpdateFailure::LibraryAnalysis);
                    }
                }
            }
        }
        // Transcodes whose waveform bundle is gone: drawn from their file.
        for update in &mut updates {
            let Some((fault, WaveformSource::Transcode { dir, file })) = &update.waveform else {
                continue;
            };
            if crate::service::transcode::write_transcode_waveform_bundle(dir, file) {
                let dat = crate::service::transcode::transcode_waveform_bundle(dir).dat_path;
                update.waveform = Some((*fault, WaveformSource::Library(dat)));
            } else {
                failures.insert(update.track_id, UpdateFailure::Unreadable);
            }
        }

        // The stick's own audio, analysed in the one analysis queue: for a
        // track getting its own bundle, and for a waveform with no library
        // one.
        let draws = updates
            .iter()
            .filter(|u| !failures.contains_key(&u.track_id))
            .filter(|u| u.own_bundle || matches!(u.waveform, Some((_, WaveformSource::Draw))))
            .filter_map(|u| {
                let row = row_by_id.get(&u.track_id)?;
                let audio = crate::service::usb::root::resolve_usb_side_path(
                    usb_root,
                    &row.track_file_path,
                )
                .filter(|path| path.is_file());
                let bpm = (row.tempo_x100 > 0).then(|| f64::from(row.tempo_x100) / 100.0);
                Some((u.track_id, audio, bpm))
            })
            .collect::<Vec<_>>();
        let mut analyses = HashMap::new();
        let mut queued = Vec::new();
        for (id, audio, kept_bpm) in draws {
            match audio {
                Some(audio) => queued.push((id, audio, kept_bpm)),
                None => {
                    analyses.insert(id, (None, Err(DrawError::Unreadable)));
                }
            }
        }
        let results = self.analyze_stick_files(
            queued
                .iter()
                .map(|(_, audio, kept_bpm)| StickAudio {
                    audio: audio.clone(),
                    kept_bpm: *kept_bpm,
                    detect_key: false,
                })
                .collect(),
        )?;
        for ((id, audio, _), result) in queued.into_iter().zip(results) {
            analyses.insert(id, (Some(audio), result));
        }

        let mut outcome = AppBundleOutcome::default();
        let mut mutations = Vec::<crate::pdb_writer::PdbTrackRowMutation>::new();
        // (track path, new USB-relative `.DAT` path) for the eDB.
        let mut moved = Vec::<(String, String)>::new();
        let mut meter = crate::progress::SpeedMeter::new();
        for (index, update) in updates.iter().enumerate() {
            on_event(SeekApplyEvent::ItemDone { done: index, total });
            if failures.contains_key(&update.track_id) {
                continue;
            }
            let Some(row) = row_by_id.get(&update.track_id) else {
                continue;
            };
            if update.own_bundle {
                // Written one at a time: the next free `ANLZnnnn` depends on
                // the bundles written before it.
                let Some((Some(audio), analysis)) = analyses.remove(&update.track_id) else {
                    failures.insert(update.track_id, UpdateFailure::Unreadable);
                    continue;
                };
                let analysis = match analysis {
                    Ok(analysis) => analysis,
                    Err(err) => {
                        failures.insert(update.track_id, err.into());
                        continue;
                    }
                };
                let (dat, ext, twoex) =
                    analysis_bundle_paths_for_track(usb_root, &update.track_path);
                let paths = AnlzBundlePaths {
                    dat_path: dat.clone(),
                    ext_path: ext,
                    twoex_path: twoex,
                };
                if let Err(err) = write_stick_bundle(&analysis, &paths, &update.track_path, &audio)
                {
                    let err = match err {
                        DrawError::Write(err) => err,
                        _ => BackendError::Internal("bundle not written".to_string()),
                    };
                    failures.insert(update.track_id, UpdateFailure::Write(err));
                    continue;
                }
                let Some(rel) = usb_relative_path(usb_root, &dat) else {
                    continue;
                };
                let mut data = track_row_data_from_reader_row(row);
                data.anlz_path = rel.clone();
                mutations.push(crate::pdb_writer::PdbTrackRowMutation {
                    row: data,
                    changed_fields: vec!["anlz_path"],
                });
                warnings.push(logging::log_text(
                    Level::Info,
                    "usb-repair",
                    "usb.repair.shared-analysis.separated",
                    &tr::repair_warning_analysis_separated(&update.track_path, &rel),
                ));
                moved.push((update.track_path.clone(), rel));
                outcome.separated += 1;
                continue;
            }

            let mut written = || -> Result<(bool, SeekAdded), UpdateFailure> {
                let mut changed = false;
                if let Some((_, source)) = &update.waveform {
                    let files = match source {
                        WaveformSource::Library(dat) => WaveformFiles::read(dat),
                        WaveformSource::Draw => match analyses.remove(&update.track_id) {
                            Some((_, Ok(analysis))) => WaveformFiles::drawn(&analysis),
                            Some((_, Err(err))) => return Err(err.into()),
                            None => return Err(UpdateFailure::Unreadable),
                        },
                        // Resolved above, or failed there.
                        WaveformSource::AnalyzeLibrary(_) | WaveformSource::Transcode { .. } => {
                            return Err(UpdateFailure::LibraryAnalysis);
                        }
                    };
                    changed |= files.copy_into(&update.dat).map_err(UpdateFailure::Write)?;
                }
                // Each bundle ends up in the current format, whatever it
                // was found for; a current one is left as it is.
                changed |= stamp_bundle_ppth(&update.dat, &update.track_path)
                    .map_err(UpdateFailure::Write)?;
                match upgrade_bundle_beat_grid(&update.dat).map_err(UpdateFailure::Write)? {
                    GridUpgrade::Rewritten => changed = true,
                    GridUpgrade::AlreadyCurrent => {}
                    GridUpgrade::NotRebuildable => warnings.push(logging::log_text(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.beat-grid.skipped",
                        &tr::repair_warning_beat_grid_not_constant(update.dat.display()),
                    )),
                }
                // The MP3 seek index, read from the file as the MP3 seek
                // data has always been: a file it can't be reproduced for
                // exactly is named and remembered.
                let seek = match &update.mp3_seek {
                    Some(target) => add_seek_data(
                        target,
                        SeekFormat::Mp3,
                        seek_skips,
                        warnings,
                        &mut SeekProgress {
                            done: index,
                            total,
                            meter: &mut meter,
                            on_event: &mut *on_event,
                        },
                    )
                    .map_err(UpdateFailure::Write)?,
                    None => SeekAdded::NotNeeded,
                };
                Ok((changed, seek))
            };
            match written() {
                Ok((changed, seek)) => {
                    if changed || seek == SeekAdded::Written {
                        outcome.updated += 1;
                    } else if seek == SeekAdded::Skipped {
                        outcome.skipped += 1;
                    }
                }
                Err(failure) => {
                    failures.insert(update.track_id, failure);
                }
            }
        }

        if !mutations.is_empty() {
            crate::pdb_writer::mutate_tracks_in_place(
                &mut pdb_bytes,
                &mutations,
                PdbLayoutProfile::DEFAULT,
                page_size,
            )?;
            std::fs::write(&pdb_path, &pdb_bytes)?;
        }
        if !moved.is_empty()
            && let Some(mut handle) = EdbRw::shared_or_open(edb_conn, usb_root)
            && table_exists(handle.conn(), "content")
        {
            repoint_edb_analysis_paths(handle.conn(), &moved)?;
        }

        for update in &updates {
            let Some(failure) = failures.remove(&update.track_id) else {
                continue;
            };
            outcome.failed += 1;
            let line = match failure {
                UpdateFailure::Unreadable => {
                    tr::repair_warning_app_analysis_unreadable(&update.track_path)
                }
                UpdateFailure::LibraryAnalysis => {
                    tr::repair_warning_app_analysis_library_failed(&update.track_path)
                }
                UpdateFailure::Cancelled => {
                    tr::repair_warning_analysis_cancelled(&update.track_path)
                }
                UpdateFailure::Write(err) => {
                    tr::repair_warning_app_analysis_write_failed(&update.track_path, err)
                }
            };
            warnings.push(logging::log_text(
                Level::Error,
                "usb-repair",
                "usb.repair.app-analysis.failed",
                &line,
            ));
        }
        on_event(SeekApplyEvent::ItemDone { done: total, total });
        Ok(outcome)
    }
}

/// Points the eDB content rows of `moved` (track path, new USB-relative
/// `.DAT`) at their new bundle.
fn repoint_edb_analysis_paths(
    conn: &mut rusqlite::Connection,
    moved: &[(String, String)],
) -> BackendResult<()> {
    let new_by_key = moved
        .iter()
        .map(|(track, rel)| (audio_path_key(track), rel))
        .collect::<HashMap<_, _>>();
    let tx = conn.transaction()?;
    let targets = {
        let mut stmt = tx.prepare("SELECT content_id, path FROM content WHERE path IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut targets = Vec::<(i64, String)>::new();
        for row in rows {
            let (content_id, path) = row?;
            if let Some(rel) = new_by_key.get(&audio_path_key(&path)) {
                targets.push((content_id, (*rel).clone()));
            }
        }
        targets
    };
    for (content_id, rel) in targets {
        tx.execute(
            "UPDATE content SET analysisDataFilePath = ?1 WHERE content_id = ?2",
            params![rel, content_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}
