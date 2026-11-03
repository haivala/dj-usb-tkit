//! Which file of a track goes onto a USB: the track's own, or one of its
//! transcodes, by the stick's Player formats (see `usb_identity::DriveFormats`).
//!
//! The stick's order of preference picks among the files the track has in a
//! ticked format: its own file and its up-to-date transcodes. When none is
//! ticked, a lossless track gets a transcode made by the export itself, in the
//! first ticked format the app can make; a lossy one (MP3, AAC) can't be
//! converted, so the export is refused.
//!
//! A transcode's audio starts `offset_ms` later than the track's (MP3's
//! encoder delay; 0 for AIFF/WAV), so everything timed on the track moves
//! that much later on the stick: first beat, cues, beat grid. The waveforms
//! come from the transcode itself. Coming back from the stick (a cue edited
//! on the USB row, cues imported from it), the offset is taken off again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params, params_from_iter};

use crate::audio_encode::TranscodeFormat;
use crate::audio_format::{PlayerFormat, format_ext_from_path, track_rekordbox_file_type};
use crate::edb::ExportTrackData;
use crate::error::BackendResult;
use crate::models::{Track, TrackExportFile};

use super::anlz::{atomic_write_bytes, transcode_bundle_file};
use super::transcode::{
    file_facts, size_text, transcode_bundle_dir, transcode_export_bundle,
    transcode_waveform_bundle, write_transcode_waveform_bundle,
};
use super::usb_formats::library_player_format;
use super::usb_identity::DriveFormats;

/// A recorded transcode of a track.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TranscodeFile {
    pub(crate) format: TranscodeFormat,
    pub(crate) file_path: String,
    pub(crate) file_size_bytes: i64,
    pub(crate) sample_rate_hz: Option<u32>,
    pub(crate) bit_depth: Option<u8>,
    pub(crate) bitrate_kbps: Option<u32>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) offset_ms: f64,
}

impl TranscodeFile {
    /// The offset in whole milliseconds, as cue and beat times are kept.
    pub(crate) fn shift_ms(&self) -> u32 {
        offset_to_ms(self.offset_ms)
    }
}

fn offset_to_ms(offset_ms: f64) -> u32 {
    if offset_ms.is_finite() && offset_ms > 0.0 {
        offset_ms.round() as u32
    } else {
        0
    }
}

/// What export puts on the stick for one track.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExportFile {
    /// The track's own file.
    Source,
    /// An up-to-date transcode.
    Transcode(TranscodeFile),
    /// No usable file yet: export makes a transcode in this format first.
    Make(TranscodeFormat),
    /// No ticked format the track has or can be converted to.
    Refused(PlayerFormat),
}

/// Where the "has the track's file changed since its transcode was made"
/// check reads the file's size and time from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceFacts {
    /// As the library scan recorded them: touches no music file (the
    /// playlist heading).
    Recorded,
    /// From the file itself (export).
    OnDisk,
}

/// The [`PlayerFormat`] a transcode format is.
fn player_format_of(format: TranscodeFormat) -> PlayerFormat {
    match format {
        TranscodeFormat::Aiff => PlayerFormat::Aiff,
        TranscodeFormat::Wav => PlayerFormat::Wav,
        TranscodeFormat::Mp3 => PlayerFormat::Mp3,
    }
}

/// The transcode format a [`PlayerFormat`] is, when the app can make it.
fn transcode_format_of(format: PlayerFormat) -> Option<TranscodeFormat> {
    TranscodeFormat::ALL
        .into_iter()
        .find(|&candidate| player_format_of(candidate) == format)
}

fn is_lossless(format: PlayerFormat) -> bool {
    matches!(
        format,
        PlayerFormat::Flac | PlayerFormat::Alac | PlayerFormat::Aiff | PlayerFormat::Wav
    )
}

struct StoredTranscode {
    file: TranscodeFile,
    source_size_bytes: Option<i64>,
    source_modified_at: Option<i64>,
}

/// Each of `track_ids`' transcodes whose file exists.
fn load_transcodes(
    conn: &rusqlite::Connection,
    track_ids: &[&str],
) -> BackendResult<HashMap<String, Vec<StoredTranscode>>> {
    let mut by_track = HashMap::<String, Vec<StoredTranscode>>::new();
    for chunk in track_ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT track_id, format, file_path, file_size_bytes, sample_rate_hz, bit_depth,
                    bitrate_kbps, duration_ms, offset_ms, source_size_bytes, source_modified_at
             FROM track_transcodes WHERE track_id IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            // A format this build doesn't make: not a usable transcode.
            let Some(format) = TranscodeFormat::parse(&row.get::<_, String>(1)?) else {
                return Ok(None);
            };
            Ok(Some((
                row.get::<_, String>(0)?,
                TranscodeFile {
                    format,
                    file_path: row.get(2)?,
                    file_size_bytes: row.get(3)?,
                    sample_rate_hz: row.get(4)?,
                    bit_depth: row.get(5)?,
                    bitrate_kbps: row.get(6)?,
                    duration_ms: row.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
                    offset_ms: row.get(8)?,
                },
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
            )))
        })?;
        for row in rows {
            let Some((track_id, file, source_size_bytes, source_modified_at)) = row? else {
                continue;
            };
            if !Path::new(&file.file_path).is_file() {
                continue;
            }
            by_track.entry(track_id).or_default().push(StoredTranscode {
                file,
                source_size_bytes,
                source_modified_at,
            });
        }
    }
    Ok(by_track)
}

/// A file's size and modified time (Unix seconds).
type FileFacts = (Option<i64>, Option<i64>);

/// The size and time of each track's file as the scan recorded them.
fn recorded_source_facts(
    conn: &rusqlite::Connection,
    track_ids: &[&str],
) -> BackendResult<HashMap<String, FileFacts>> {
    let mut facts = HashMap::new();
    for chunk in track_ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT id, file_size_bytes, file_modified_at FROM tracks WHERE id IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, Option<i64>>(1)?, row.get::<_, Option<i64>>(2)?),
            ))
        })?;
        for row in rows {
            let (id, fact) = row?;
            facts.insert(id, fact);
        }
    }
    Ok(facts)
}

/// What export puts on the stick for each of `tracks`, in order.
pub(crate) fn plan_export_files(
    conn: &rusqlite::Connection,
    tracks: &[ExportTrackData],
    formats: &DriveFormats,
    facts: SourceFacts,
) -> BackendResult<Vec<ExportFile>> {
    let ids: Vec<&str> = tracks.iter().map(|track| track.id.as_str()).collect();
    let mut transcodes = load_transcodes(conn, &ids)?;
    let recorded = match facts {
        SourceFacts::Recorded => recorded_source_facts(conn, &ids)?,
        SourceFacts::OnDisk => HashMap::new(),
    };
    Ok(tracks
        .iter()
        .map(|track| {
            let own = transcodes.remove(&track.id).unwrap_or_default();
            let (size, modified) = match facts {
                SourceFacts::Recorded => recorded.get(&track.id).copied().unwrap_or_default(),
                SourceFacts::OnDisk => file_facts(Path::new(&track.file_path)),
            };
            // As the library shows it (`fill_track_transcodes`): a file that
            // can't be read doesn't make its transcodes outdated.
            let up_to_date: Vec<TranscodeFile> = own
                .into_iter()
                .filter(|stored| {
                    size.is_none()
                        || (size == stored.source_size_bytes
                            && modified == stored.source_modified_at)
                })
                .map(|stored| stored.file)
                .collect();
            match choose_export_file(track, &up_to_date, formats) {
                // Nothing to convert from: export's own missing-file rule
                // skips it and says so.
                ExportFile::Make(_) | ExportFile::Refused(_)
                    if facts == SourceFacts::OnDisk && !Path::new(&track.file_path).is_file() =>
                {
                    ExportFile::Source
                }
                file => file,
            }
        })
        .collect())
}

fn choose_export_file(
    track: &ExportTrackData,
    up_to_date: &[TranscodeFile],
    formats: &DriveFormats,
) -> ExportFile {
    // Not a Player formats format: export's own "players don't play it"
    // rule deals with it.
    let Some(own) = library_player_format(&track.file_path, track.bit_depth) else {
        return ExportFile::Source;
    };
    let ticked = || {
        formats
            .entries()
            .iter()
            .filter(|(_, allowed)| *allowed)
            .map(|&(format, _)| format)
    };
    for format in ticked() {
        if format == own {
            return ExportFile::Source;
        }
        if let Some(file) = up_to_date
            .iter()
            .find(|file| player_format_of(file.format) == format)
        {
            return ExportFile::Transcode(file.clone());
        }
    }
    if !is_lossless(own) {
        return ExportFile::Refused(own);
    }
    ticked()
        .find_map(transcode_format_of)
        .map_or(ExportFile::Refused(own), ExportFile::Make)
}

/// What the FMT column shows for `file`; `None` for the track's own file.
fn track_export_file(track: &Track, file: &ExportFile) -> Option<TrackExportFile> {
    let source = format!("source: {}", track_format_text(track));
    let (kind, badge, tooltip) = match file {
        ExportFile::Source => return None,
        ExportFile::Transcode(file) => (
            "transcode",
            file.format.as_str().to_ascii_uppercase(),
            format!(
                "{} transcode · {}, exported to this USB — {source}",
                file.format.label(),
                size_text(file.file_size_bytes)
            ),
        ),
        ExportFile::Make(format) => (
            "convert",
            format.as_str().to_ascii_uppercase(),
            format!("Converted to {} on export — {source}", format.label()),
        ),
        ExportFile::Refused(format) => (
            "refused",
            format.label().to_string(),
            format!(
                "{} — {} can't go on this USB: only lossless files can be converted",
                track_format_text(track),
                format.label()
            ),
        ),
    };
    Some(TrackExportFile {
        kind: kind.to_string(),
        badge,
        tooltip,
    })
}

/// A track's own file as the FMT column describes it: `"FLAC · 44.1 kHz ·
/// 16-bit"`, `"MP3 · 44.1 kHz · 320 kbps"`.
fn track_format_text(track: &Track) -> String {
    let format = track
        .format_ext
        .as_deref()
        .filter(|ext| !ext.is_empty())
        .map_or_else(|| "Unknown".to_string(), str::to_ascii_uppercase);
    let mut parts = vec![format];
    if let Some(rate) = track.sample_rate_hz.filter(|&rate| rate > 0) {
        let khz = f64::from(rate) / 1000.0;
        parts.push(if khz.fract() == 0.0 {
            format!("{khz:.0} kHz")
        } else {
            format!("{khz:.1} kHz")
        });
    }
    if let Some(depth) = track.bit_depth.filter(|&depth| depth > 0) {
        parts.push(format!("{depth}-bit"));
    } else if let Some(bitrate) = track.bitrate_kbps.filter(|&rate| rate > 0) {
        parts.push(format!("{bitrate} kbps"));
    }
    parts.join(" · ")
}

/// Sets each of `tracks`' `export_file` for the USB at `usb_root`, by its
/// Player formats, from recorded data only (no music file is read), and
/// marks the transcode that goes as `exported`.
pub(crate) fn fill_track_export_files(
    conn: &rusqlite::Connection,
    tracks: &mut [Track],
    usb_root: &Path,
) -> BackendResult<()> {
    let export_tracks: Vec<ExportTrackData> = tracks
        .iter()
        .map(|track| ExportTrackData {
            id: track.id.clone(),
            file_path: track.file_path.clone(),
            bit_depth: track.bit_depth,
            ..Default::default()
        })
        .collect();
    let formats = super::usb_identity::read_drive_formats(usb_root);
    let plan = plan_export_files(conn, &export_tracks, &formats, SourceFacts::Recorded)?;
    for (track, file) in tracks.iter_mut().zip(plan) {
        if let ExportFile::Transcode(file) = &file {
            for transcode in &mut track.transcodes {
                transcode.exported = transcode.file_path == file.file_path;
            }
        }
        track.export_file = track_export_file(track, &file);
    }
    Ok(())
}

/// `track` as its transcode `file`: the file's facts, and its first beat and
/// cues moved onto the transcode's timeline. The analysis bundle is left to
/// [`prepare_transcode_bundle`].
pub(crate) fn use_transcode(track: &mut ExportTrackData, file: &TranscodeFile) {
    let shift = file.shift_ms();
    track.file_path = file.file_path.clone();
    track.file_name = Path::new(&file.file_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| track.file_name.clone());
    track.file_size_bytes = Some(file.file_size_bytes);
    track.sample_rate_hz = file.sample_rate_hz;
    track.bit_depth = file.bit_depth;
    track.bitrate_kbps = file.bitrate_kbps;
    track.transcode_format = Some(file.format.as_str());
    track.file_type = Some(track_rekordbox_file_type(
        file.format.extension(),
        file.bit_depth,
    ));
    if let Some(duration_ms) = file.duration_ms {
        track.duration_ms = Some(duration_ms);
    }
    if shift > 0 {
        track.first_beat_ms = track.first_beat_ms.map(|ms| ms.saturating_add(shift));
        for cue in &mut track.cues {
            cue.position_ms = cue.position_ms.saturating_add(shift);
        }
    }
}

/// About how big a transcode of `track` into `format` will be.
pub(crate) fn estimated_transcode_bytes(track: &ExportTrackData, format: TranscodeFormat) -> u64 {
    let seconds = track.duration_ms.unwrap_or(0) as f64 / 1000.0;
    let bytes_per_second = match format {
        TranscodeFormat::Mp3 => 320_000.0 / 8.0,
        TranscodeFormat::Aiff | TranscodeFormat::Wav => {
            let rate = f64::from(track.sample_rate_hz.unwrap_or(44_100));
            let depth = f64::from(track.bit_depth.filter(|&d| d > 0).unwrap_or(16));
            rate * depth / 8.0 * 2.0
        }
    };
    (seconds * bytes_per_second) as u64
}

/// The analysis bundle export copies for a transcode: the track's own bundle
/// (`track_bundle`, its `.DAT` path) with the transcode's waveforms and seek
/// index and its beat grid moved `shift_ms` later, written next to the
/// transcode's waveform bundle. Draws that waveform bundle first when it is
/// missing (a transcode made before export used them). Returns the `.DAT`
/// path; `None` when either bundle can't be had.
pub(crate) fn prepare_transcode_bundle(
    data_dir: &Path,
    track_bundle: &str,
    transcode: &TranscodeFile,
) -> Option<String> {
    let transcode_path = PathBuf::from(&transcode.file_path);
    let dir = transcode_bundle_dir(data_dir, &transcode_path);
    let waveform = transcode_waveform_bundle(&dir);
    if (!waveform.dat_path.is_file() || !waveform.ext_path.is_file())
        && !write_transcode_waveform_bundle(&dir, &transcode_path)
    {
        return None;
    }
    let out = transcode_export_bundle(&dir);
    let track_bundle = Path::new(track_bundle);
    for (ext, from, to) in [
        ("DAT", &waveform.dat_path, &out.dat_path),
        ("EXT", &waveform.ext_path, &out.ext_path),
        ("2EX", &waveform.twoex_path, &out.twoex_path),
    ] {
        let transcode_bytes = std::fs::read(from).ok();
        let merged = match std::fs::read(track_bundle.with_extension(ext)) {
            Ok(track_bytes) => transcode_bundle_file(
                &track_bytes,
                transcode_bytes.as_deref()?,
                transcode.shift_ms(),
            ),
            // The track's `.2EX` is optional (rekordbox imports may lack
            // one); it holds only waveforms, so the transcode's own does.
            Err(_) if ext == "2EX" => transcode_bytes,
            Err(_) => None,
        };
        match merged {
            Some(bytes) => {
                if std::fs::read(to).ok().as_deref() != Some(bytes.as_slice()) {
                    atomic_write_bytes(to, &bytes).ok()?;
                }
            }
            None if ext == "2EX" => {
                let _ = std::fs::remove_file(to);
            }
            None => return None,
        }
    }
    Some(out.dat_path.to_string_lossy().to_string())
}

/// How much later than the track's own the audio of `usb_file` (a file on a
/// stick) starts, when it is one of `track_id`'s transcodes: same format and
/// same size. `None` when it isn't one.
pub(crate) fn transcode_offset_for_usb_file(
    conn: &rusqlite::Connection,
    track_id: &str,
    usb_file: &Path,
) -> BackendResult<Option<u32>> {
    let Some(format) = format_ext_from_path(&usb_file.to_string_lossy())
        .as_deref()
        .and_then(TranscodeFormat::parse)
    else {
        return Ok(None);
    };
    let Some(size) = std::fs::metadata(usb_file)
        .ok()
        .and_then(|meta| i64::try_from(meta.len()).ok())
    else {
        return Ok(None);
    };
    let offset: Option<f64> = conn
        .query_row(
            "SELECT offset_ms FROM track_transcodes
             WHERE track_id = ?1 AND format = ?2 AND file_size_bytes = ?3",
            params![track_id, format.as_str(), size],
            |row| row.get(0),
        )
        .optional()?;
    Ok(offset.map(offset_to_ms))
}

/// The tracks with a transcode in `format_ext`'s format of exactly `size`
/// bytes and about `duration_ms` long: a file on a stick like that is one of
/// theirs. The same size and length rule as a track's own file
/// (`library_metadata_candidate_compatible`), against the transcode's own;
/// an unknown length on either side doesn't rule it out.
pub(crate) fn track_ids_with_transcode(
    conn: &rusqlite::Connection,
    format_ext: &str,
    size: i64,
    duration_ms: Option<i64>,
) -> BackendResult<Vec<String>> {
    let Some(format) = TranscodeFormat::parse(format_ext) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare_cached(
        "SELECT track_id FROM track_transcodes
         WHERE format = ?1 AND file_size_bytes = ?2
           AND (?3 IS NULL OR duration_ms IS NULL OR ABS(duration_ms - ?3) <= ?4)",
    )?;
    let rows = stmt.query_map(
        params![
            format.as_str(),
            size,
            duration_ms,
            super::track_matching::METADATA_DURATION_TOLERANCE_MS
        ],
        |row| row.get(0),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TrackCue;

    fn formats(order: &[(&str, bool)]) -> DriveFormats {
        let items: Vec<(String, bool)> = order
            .iter()
            .map(|(id, allowed)| (id.to_string(), *allowed))
            .collect();
        DriveFormats::from_request(&items).unwrap()
    }

    fn no(unticked: &[&str]) -> DriveFormats {
        let items: Vec<(String, bool)> = PlayerFormat::DEFAULT_ORDER
            .into_iter()
            .map(|format| (format.id().to_string(), !unticked.contains(&format.id())))
            .collect();
        DriveFormats::from_request(&items).unwrap()
    }

    fn track(id: &str, file_path: &str, bit_depth: Option<u8>) -> ExportTrackData {
        ExportTrackData {
            id: id.to_string(),
            file_path: file_path.to_string(),
            bit_depth,
            ..Default::default()
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        conn: rusqlite::Connection,
        paths: HashMap<&'static str, String>,
    }

    /// A DB with `tracks` (size 100, time 5) and the given transcodes
    /// (track id, format, recorded source size); every transcode file
    /// exists except `gone`'s.
    fn fixture(transcodes: &[(&'static str, &'static str, i64)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tracks (id TEXT, file_size_bytes INTEGER, file_modified_at INTEGER);
             CREATE TABLE track_transcodes (track_id TEXT, format TEXT, file_path TEXT,
               file_size_bytes INTEGER, sample_rate_hz INTEGER, bit_depth INTEGER,
               bitrate_kbps INTEGER, duration_ms INTEGER, offset_ms REAL,
               source_size_bytes INTEGER, source_modified_at INTEGER);",
        )
        .unwrap();
        let mut paths = HashMap::new();
        for (index, (id, format, source_size)) in transcodes.iter().enumerate() {
            conn.execute("INSERT INTO tracks VALUES (?1, 100, 5)", params![id])
                .unwrap();
            let path = dir.path().join(format!("{id}-{index}.{format}"));
            if *id != "gone" {
                std::fs::write(&path, b"x").unwrap();
            }
            let path = path.to_string_lossy().to_string();
            conn.execute(
                "INSERT INTO track_transcodes VALUES
                   (?1, ?2, ?3, 1234, 44100, NULL, 320, 1000, 25.06, ?4, 5)",
                params![id, format, path, source_size],
            )
            .unwrap();
            paths.insert(*id, path);
        }
        Fixture {
            _dir: dir,
            conn,
            paths,
        }
    }

    fn plan(
        fixture: &Fixture,
        tracks: &[ExportTrackData],
        formats: &DriveFormats,
    ) -> Vec<ExportFile> {
        plan_export_files(&fixture.conn, tracks, formats, SourceFacts::Recorded).unwrap()
    }

    #[test]
    fn picks_by_the_sticks_order_and_makes_or_refuses_the_rest() {
        let fx = fixture(&[("a", "mp3", 100), ("b", "mp3", 99), ("gone", "mp3", 100)]);
        let tracks = [
            track("a", "/m/a.flac", Some(16)),
            track("b", "/m/b.flac", Some(16)),
            track("gone", "/m/gone.flac", Some(16)),
            track("lossy", "/m/lossy.m4a", None),
            track("mp3", "/m/x.mp3", None),
            track("ogg", "/m/x.ogg", None),
        ];
        let got = plan(&fx, &tracks, &no(&["flac", "aac"]));
        let ExportFile::Transcode(file) = &got[0] else {
            panic!("{:?}", got[0]);
        };
        assert_eq!(file.file_path, fx.paths["a"]);
        assert_eq!(file.shift_ms(), 25);
        // b's transcode is outdated, gone's file is missing: made again, in
        // the first ticked format the app makes (AIFF, by the default order).
        assert_eq!(got[1], ExportFile::Make(TranscodeFormat::Aiff));
        assert_eq!(got[2], ExportFile::Make(TranscodeFormat::Aiff));
        assert_eq!(got[3], ExportFile::Refused(PlayerFormat::Aac));
        assert_eq!(got[4], ExportFile::Source);
        assert_eq!(got[5], ExportFile::Source);

        // A ticked own format before the transcode's: the track's own file.
        let flac_first = formats(&[
            ("flac", true),
            ("mp3", true),
            ("alac", false),
            ("aiff", false),
            ("wav", false),
            ("aac", false),
        ]);
        assert_eq!(plan(&fx, &tracks[..1], &flac_first)[0], ExportFile::Source);
        // The transcode's format first: the transcode, though FLAC is ticked.
        let mp3_first = formats(&[
            ("mp3", true),
            ("flac", true),
            ("alac", false),
            ("aiff", false),
            ("wav", false),
            ("aac", false),
        ]);
        assert!(matches!(
            plan(&fx, &tracks[..1], &mp3_first)[0],
            ExportFile::Transcode(_)
        ));
        // Only formats the app can't make are ticked: refused.
        let only_alac = formats(&[
            ("alac", true),
            ("flac", false),
            ("mp3", false),
            ("aiff", false),
            ("wav", false),
            ("aac", false),
        ]);
        assert_eq!(
            plan(&fx, &tracks[1..2], &only_alac)[0],
            ExportFile::Refused(PlayerFormat::Flac)
        );
    }

    #[test]
    fn a_missing_file_with_nothing_to_use_is_left_to_the_missing_file_rule() {
        let fx = fixture(&[("a", "mp3", 100)]);
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().join("here.flac");
        std::fs::write(&here, b"x").unwrap();
        let tracks = [
            // Its file is gone, its MP3 is still usable.
            track("a", "/gone/a.flac", Some(16)),
            track("flac", "/gone/b.flac", Some(16)),
            track("aac", "/gone/c.m4a", None),
            track("here", &here.to_string_lossy(), Some(16)),
        ];
        let got = plan_export_files(
            &fx.conn,
            &tracks,
            &no(&["flac", "aac"]),
            SourceFacts::OnDisk,
        )
        .unwrap();
        assert!(matches!(got[0], ExportFile::Transcode(_)), "{:?}", got[0]);
        assert_eq!(got[1], ExportFile::Source);
        assert_eq!(got[2], ExportFile::Source);
        assert_eq!(got[3], ExportFile::Make(TranscodeFormat::Aiff));
    }

    #[test]
    fn a_stick_file_is_a_transcode_by_format_size_and_length() {
        // Each transcode: 1234 bytes, 1000 ms.
        let fx = fixture(&[("a", "mp3", 100), ("b", "wav", 100)]);
        let ids = |ext: &str, size: i64, duration: Option<i64>| {
            track_ids_with_transcode(&fx.conn, ext, size, duration).unwrap()
        };
        assert_eq!(ids("mp3", 1234, Some(1000)), vec!["a".to_string()]);
        assert_eq!(ids("mp3", 1234, Some(2900)), vec!["a".to_string()]);
        assert_eq!(ids("mp3", 1234, None), vec!["a".to_string()]);
        assert!(ids("mp3", 1234, Some(3100)).is_empty());
        assert!(ids("mp3", 1235, Some(1000)).is_empty());
        assert!(ids("aiff", 1234, Some(1000)).is_empty());
        assert!(ids("flac", 1234, Some(1000)).is_empty());
    }

    #[test]
    fn the_fmt_column_text_names_what_goes_and_the_source() {
        let mut track = crate::service::tests::sample_track("t", "/m/a.flac");
        track.format_ext = Some("flac".to_string());
        track.sample_rate_hz = Some(44_100);
        track.bit_depth = Some(16);
        let shown = |track: &Track, file: ExportFile| {
            track_export_file(track, &file).map(|f| (f.kind, f.badge, f.tooltip))
        };

        assert_eq!(shown(&track, ExportFile::Source), None);
        assert_eq!(
            shown(&track, ExportFile::Make(TranscodeFormat::Mp3)),
            Some((
                "convert".to_string(),
                "MP3".to_string(),
                "Converted to MP3 320 on export — source: FLAC · 44.1 kHz · 16-bit".to_string()
            ))
        );
        let file = TranscodeFile {
            format: TranscodeFormat::Aiff,
            file_path: "/t/a.aiff".to_string(),
            file_size_bytes: 9 * 1024 * 1024,
            sample_rate_hz: Some(48_000),
            bit_depth: Some(24),
            bitrate_kbps: None,
            duration_ms: None,
            offset_ms: 0.0,
        };
        assert_eq!(
            shown(&track, ExportFile::Transcode(file)),
            Some((
                "transcode".to_string(),
                "AIFF".to_string(),
                "AIFF transcode · 9 MB, exported to this USB — source: FLAC · 44.1 kHz · 16-bit"
                    .to_string()
            ))
        );

        track.format_ext = Some("m4a".to_string());
        track.sample_rate_hz = Some(48_000);
        track.bit_depth = None;
        track.bitrate_kbps = Some(256);
        assert_eq!(
            shown(&track, ExportFile::Refused(PlayerFormat::Aac)),
            Some((
                "refused".to_string(),
                "AAC".to_string(),
                "M4A · 48 kHz · 256 kbps — AAC can't go on this USB: only lossless files can be \
                 converted"
                    .to_string()
            ))
        );
    }

    #[test]
    fn using_a_transcode_moves_the_timed_things_by_its_offset() {
        let mut track = ExportTrackData {
            file_path: "/m/a.flac".to_string(),
            file_name: "a.flac".to_string(),
            file_size_bytes: Some(9_000),
            bit_depth: Some(24),
            file_type: Some(5),
            first_beat_ms: Some(100),
            cues: vec![TrackCue {
                id: "c".to_string(),
                position_ms: 1_000,
                color_id: None,
                name: None,
                playback_start: false,
                memory: false,
            }],
            ..Default::default()
        };
        let file = TranscodeFile {
            format: TranscodeFormat::Mp3,
            file_path: "/t/a.mp3".to_string(),
            file_size_bytes: 1_234,
            sample_rate_hz: Some(44_100),
            bit_depth: None,
            bitrate_kbps: Some(320),
            duration_ms: Some(60_000),
            offset_ms: 25.06,
        };
        use_transcode(&mut track, &file);
        assert_eq!(track.file_path, "/t/a.mp3");
        assert_eq!(track.file_name, "a.mp3");
        assert_eq!(track.file_size_bytes, Some(1_234));
        assert_eq!(track.bit_depth, None);
        assert_eq!(track.file_type, Some(1));
        assert_eq!(track.duration_ms, Some(60_000));
        assert_eq!(track.first_beat_ms, Some(125));
        assert_eq!(track.cues[0].position_ms, 1_025);
        assert_eq!(track.transcode_format, Some("mp3"));

        let mut wav = ExportTrackData {
            first_beat_ms: Some(100),
            ..Default::default()
        };
        use_transcode(
            &mut wav,
            &TranscodeFile {
                format: TranscodeFormat::Wav,
                offset_ms: 0.0,
                bit_depth: Some(24),
                ..file
            },
        );
        assert_eq!(wav.first_beat_ms, Some(100));
        assert_eq!(wav.file_type, Some(11));
    }
}
