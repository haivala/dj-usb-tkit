//! Write the Engine DJ library for a USB from what is already on it: tracks
//! and playlists from `export.pdb`, beat grid, cues and waveform from each
//! track's ANLZ bundle, artwork from `PIONEER/Artwork`. Rebuilds `m.db`
//! every time; keeps the player's history (`hm.db`) and the track ids it
//! refers to. Format layer: [`crate::engine_db`].

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::engine_db::{
    EngineAlbumArt, EngineCue, EngineLibrary, EnginePlaylist, EngineTrack, GridMarker,
    HOT_CUE_SLOTS, OVERVIEW_ENTRIES, engine_db_dir, engine_key_from_camelot, ensure_aux_dbs,
    read_pdb_import_counter, read_previous_library, write_engine_m_db,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{DiagCheck, DiagSection, DiagStatus, TrackCue, WarningEntry};
use crate::pdb_reader::{ParsedPdb, PdbTrackRow, parse_pdb_bytes};

use super::anlz::pqtz_beats;
use super::cues::{DEFAULT_HOTCUE_COLOR_ID, HOTCUE_PALETTE, collapse_anlz_cues};
use super::key_notation::camelot_position;
use super::usb_staging;
use super::usb_utils::{find_anlz_chunk_payload, resolve_usb_side_path};
use super::{BackendService, SETTING_UI_EXPORT_ENGINE_LIBRARY, now};

/// `AlbumArt` row for tracks without artwork (every track points at a row,
/// as on a device-written library).
const NO_ART_ID: i64 = 1;

/// What [`rebuild_engine_library`] wrote.
#[derive(Debug, Clone)]
pub struct EngineLibrarySummary {
    pub tracks: usize,
    /// Tracks written with a beat grid and waveform (the player analyzes the
    /// rest on load).
    pub analyzed_tracks: usize,
    pub playlists: usize,
    pub warnings: Vec<WarningEntry>,
}

impl BackendService {
    /// Rebuild the Engine DJ library after anything rewrote `export.pdb` or an
    /// ANLZ bundle, unless the setting is off. Every PDB write bumps its
    /// sequence, and a library left on the old one is stale or re-imported by
    /// the player. The rekordbox side is already written by then, so a failure
    /// only warns.
    pub(crate) fn refresh_engine_library(&self, usb_root: &Path) -> Vec<WarningEntry> {
        if !self.engine_library_enabled() {
            return Vec::new();
        }
        match rebuild_engine_library(usb_root) {
            Ok(summary) => {
                let mut warnings = summary.warnings;
                warnings.push(logging::log(
                    Level::Info,
                    "engine-export",
                    "engine.library-written",
                    format!(
                        "Engine DJ library written (tracks: {}, analyzed: {}, playlists: {})",
                        summary.tracks, summary.analyzed_tracks, summary.playlists
                    ),
                ));
                warnings
            }
            Err(err) => vec![logging::log(
                Level::Warn,
                "engine-export",
                "engine.library-skipped",
                format!("Engine DJ library not written: {err}"),
            )],
        }
    }

    /// Off until the user turns it on (in Settings, or with the diagnostics
    /// fix offered when a USB already has an Engine library).
    pub(crate) fn engine_library_enabled(&self) -> bool {
        let Ok(conn) = self.db.connect() else {
            return false;
        };
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_EXPORT_ENGINE_LIBRARY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some_and(|value| value.trim() == "1")
    }

    pub(crate) fn enable_engine_library(&self) -> BackendResult<()> {
        self.db.connect()?.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, '1', ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![SETTING_UI_EXPORT_ENGINE_LIBRARY, now()],
        )?;
        Ok(())
    }

    /// Whether the USB has an Engine library this app should take over: the
    /// setting is off, or the library is behind `export.pdb`.
    pub(crate) fn engine_library_needs_fix(&self, usb_root: &Path) -> bool {
        engine_library_status(usb_root)
            .is_some_and(|status| !self.engine_library_enabled() || !status.in_sync())
    }

    /// Health & Diagnostics section. Without a library on the USB it's a
    /// passing note while the setting is off (so Pioneer-only users aren't
    /// warned), and `None` while it's on (the next change writes one).
    pub(crate) fn engine_library_section(&self, usb_root: &Path) -> Option<DiagSection> {
        let enabled = self.engine_library_enabled();
        let Some(status) = engine_library_status(usb_root) else {
            return (!enabled).then(|| DiagSection {
                title: "Engine DJ Library".into(),
                status: DiagStatus::Pass,
                checks: vec![DiagCheck {
                    label: "Not on this USB".into(),
                    status: DiagStatus::Pass,
                    detail: "Only needed for Denon Prime / Numark Mixstream players. Turn on \
                             Settings → Export → \"Write Engine DJ library\" before using this USB \
                             in one; otherwise the player imports export.pdb itself and analyzes \
                             every track again"
                        .into(),
                    link: None,
                }],
                counts: None,
            });
        };
        let upkeep = if enabled {
            DiagCheck {
                label: "Kept up to date".into(),
                status: DiagStatus::Pass,
                detail: "Rebuilt whenever this app changes the USB".into(),
                link: None,
            }
        } else {
            DiagCheck {
                label: "Kept up to date".into(),
                status: DiagStatus::Warn,
                detail: "\"Write Engine DJ library\" is off, so changes this app makes to the USB \
                         don't reach Denon Prime / Numark Mixstream players. Preview Fixes can turn it on"
                    .into(),
                link: None,
            }
        };
        let sync = match (status.library_counter, status.pdb_sequence) {
            (Some(library), Some(pdb)) if library == i64::from(pdb) => DiagCheck {
                label: "Matches export.pdb".into(),
                status: DiagStatus::Pass,
                detail: format!("Written for export.pdb #{pdb}"),
                link: None,
            },
            (library, pdb) => DiagCheck {
                label: "Matches export.pdb".into(),
                status: DiagStatus::Warn,
                detail: format!(
                    "Written for export.pdb #{}, the USB has #{}. The player may import \
                     export.pdb again and analyze the tracks itself. Preview Fixes can rebuild it",
                    library.map_or_else(|| "?".to_string(), |n| n.to_string()),
                    pdb.map_or_else(|| "?".to_string(), |n| n.to_string()),
                ),
                link: None,
            },
        };
        let checks = vec![upkeep, sync];
        let worst = checks.iter().fold(DiagStatus::Pass, |acc, c| {
            DiagStatus::worst(&acc, &c.status)
        });
        Some(DiagSection {
            title: "Engine DJ Library".into(),
            status: worst,
            checks,
            counts: None,
        })
    }
}

/// An Engine library on a USB, against the `export.pdb` next to it.
pub(crate) struct EngineLibraryStatus {
    pub library_counter: Option<i64>,
    pub pdb_sequence: Option<u32>,
}

impl EngineLibraryStatus {
    pub fn in_sync(&self) -> bool {
        matches!(
            (self.library_counter, self.pdb_sequence),
            (Some(library), Some(pdb)) if library == i64::from(pdb)
        )
    }
}

/// `None` when the USB has no Engine library (`Database2/m.db`).
pub(crate) fn engine_library_status(usb_root: &Path) -> Option<EngineLibraryStatus> {
    let m_db = engine_db_dir(usb_root).join("m.db");
    if !m_db.is_file() {
        return None;
    }
    let pdb_sequence = usb_staging::stage_pdb(usb_root)
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| pdb_sequence(&bytes));
    Some(EngineLibraryStatus {
        library_counter: read_pdb_import_counter(&m_db),
        pdb_sequence,
    })
}

/// The `export.pdb` header sequence (u32 LE at offset 20), which every write
/// bumps and the player compares with `m.db`.
fn pdb_sequence(pdb: &[u8]) -> Option<u32> {
    pdb.get(20..24)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Regenerate `Engine Library/Database2/m.db` (and any missing companion
/// databases) from the USB's rekordbox export.
pub fn rebuild_engine_library(usb_root: &Path) -> BackendResult<EngineLibrarySummary> {
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let pdb_bytes = std::fs::read(&pdb_path)?;
    let pdb_sequence = pdb_sequence(&pdb_bytes)
        .ok_or_else(|| BackendError::Validation("export.pdb is too short".into()))?;
    let pdb = parse_pdb_bytes(&pdb_bytes)?;

    let db_dir = engine_db_dir(usb_root);
    let previous = read_previous_library(&db_dir.join("m.db")).unwrap_or_default();
    let db_uuid = if previous.db_uuid.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        previous.db_uuid.clone()
    };

    let mut warnings = Vec::<WarningEntry>::new();
    let mut next_id = previous.track_seq + 1;
    let mut art = ArtCollector::new(usb_root);
    let mut engine_ids = HashMap::<u32, i64>::new();
    let mut tracks = Vec::with_capacity(pdb.tracks.len());
    let mut pdb_tracks: Vec<&PdbTrackRow> = pdb.tracks.iter().collect();
    pdb_tracks.sort_by_key(|t| t.id);
    for row in pdb_tracks {
        let rel = row.track_file_path.trim().trim_start_matches('/');
        if rel.is_empty() || engine_ids.contains_key(&row.id) {
            continue;
        }
        let path = format!("../{rel}");
        let id = match previous.track_ids_by_path.get(&path) {
            Some(id) => *id,
            None => {
                next_id += 1;
                next_id - 1
            }
        };
        engine_ids.insert(row.id, id);
        let mut track = engine_track(usb_root, &pdb, row, id, path);
        track.album_art_id = Some(art.id_for(&pdb, row.artwork_id));
        track.played = previous.played_by_path.get(&track.path).cloned();
        tracks.push(track);
    }
    if engine_ids.len() != engine_ids.values().collect::<HashSet<_>>().len() {
        return Err(BackendError::Validation(
            "two USB tracks share one file path".into(),
        ));
    }

    let analyzed_tracks = tracks.iter().filter(|t| t.is_analyzed()).count();
    if analyzed_tracks < tracks.len() {
        warnings.push(logging::log(
            Level::Warn,
            "engine-export",
            "engine.tracks-without-analysis",
            format!(
                "{} of {} tracks have no usable beat grid or waveform on the USB; \
                 the Engine player analyzes them itself on load",
                tracks.len() - analyzed_tracks,
                tracks.len()
            ),
        ));
    }

    let playlists = engine_playlists(&pdb, &engine_ids);
    let library = EngineLibrary {
        db_uuid,
        pdb_sequence,
        current_played_indicator: previous.current_played_indicator,
        track_id_floor: previous.track_seq + 1,
        tracks,
        playlists,
        album_art: art.into_rows(),
    };
    write_engine_m_db(&db_dir, &library)?;
    ensure_aux_dbs(&db_dir, &library.db_uuid)?;

    Ok(EngineLibrarySummary {
        tracks: library.tracks.len(),
        analyzed_tracks,
        playlists: library.playlists.len(),
        warnings,
    })
}

fn engine_track(
    usb_root: &Path,
    pdb: &ParsedPdb,
    row: &PdbTrackRow,
    id: i64,
    path: String,
) -> EngineTrack {
    let name = |map: &HashMap<u32, String>, id: u32| {
        map.get(&id)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let filename = row
        .file_name
        .clone()
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or_default().to_string());
    let sample_rate = row.sample_rate_hz.map(f64::from);
    let sample_count = sample_rate
        .zip(row.duration_seconds)
        .map(|(rate, secs)| (rate * f64::from(secs)).round() as i64);
    let key = name(&pdb.keys, row.key_id)
        .and_then(|k| camelot_position(&k))
        .and_then(|(number, minor)| engine_key_from_camelot(number, minor));

    let mut track = EngineTrack {
        id,
        file_type: Path::new(&path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase()),
        path,
        filename,
        title: Some(row.title.clone()).filter(|t| !t.is_empty()),
        artist: name(&pdb.artists, row.artist_id),
        album: name(&pdb.albums, row.album_id),
        genre: name(&pdb.genres, row.genre_id),
        comment: row.dj_comment.clone(),
        label: name(&pdb.labels, row.label_id),
        composer: name(&pdb.artists, row.composer_id),
        remixer: name(&pdb.artists, row.remixer_id),
        year: row.release_year.map(i64::from),
        length_seconds: row.duration_seconds.map(i64::from),
        bitrate_kbps: row.bitrate_kbps.map(i64::from),
        file_bytes: row.file_size_bytes.map(i64::from),
        bpm: (row.tempo_x100 > 0).then(|| f64::from(row.tempo_x100) / 100.0),
        key,
        rating: i64::from(row.rating.min(5)) * 20,
        date_added: row.date_added.as_deref().and_then(date_to_epoch),
        sample_rate,
        sample_count,
        hot_cues: vec![None; HOT_CUE_SLOTS],
        ..EngineTrack::default()
    };

    let Some(dat_path) = resolve_usb_side_path(usb_root, &row.anlz_path) else {
        return track;
    };
    let dat = std::fs::read(&dat_path).unwrap_or_default();
    let ext = std::fs::read(dat_path.with_extension("EXT")).unwrap_or_default();
    let (Some(rate), Some(count)) = (sample_rate, sample_count) else {
        return track;
    };
    if let Some(beats) = pqtz_beats(&dat) {
        track.grid = engine_grid(&beats, rate, count);
    }
    track.overview = overview_from_ext(&ext);

    let cues = collapse_anlz_cues(if ext.is_empty() { &dat } else { &ext });
    // Unrounded, as the device converts them.
    let to_samples = |ms: u32| f64::from(ms) * rate / 1000.0;
    for (slot, cue) in cues
        .iter()
        .filter(|c| c.is_hot())
        .take(HOT_CUE_SLOTS)
        .enumerate()
    {
        track.hot_cues[slot] = Some(EngineCue {
            label: cue
                .name
                .clone()
                .unwrap_or_else(|| format!("Cue {}", slot + 1)),
            sample_offset: to_samples(cue.position_ms),
            argb: engine_cue_argb(cue),
        });
    }
    // Where the player parks on load: the playback-start cue, else the first
    // memory cue, else the first beat.
    track.main_cue = cues
        .iter()
        .find(|c| c.playback_start)
        .or_else(|| cues.iter().find(|c| c.memory))
        .map(|c| to_samples(c.position_ms))
        .or_else(|| {
            pqtz_beats(&dat)
                .and_then(|b| b.first().copied())
                .map(|(_, _, ms)| to_samples(ms))
        });
    track
}

/// The rekordbox palette colour as ARGB. A Mixstream Pro snaps some colours
/// to its own palette on import; those use its values.
fn engine_cue_argb(cue: &TrackCue) -> [u8; 4] {
    let id = cue.color_id.unwrap_or(DEFAULT_HOTCUE_COLOR_ID);
    let rgb = HOTCUE_PALETTE
        .iter()
        .find(|e| e.id == id)
        .or_else(|| {
            HOTCUE_PALETTE
                .iter()
                .find(|e| e.id == DEFAULT_HOTCUE_COLOR_ID)
        })
        .map(|e| e.rgb)
        .unwrap_or((0x4E, 0xB6, 0x48));
    let (r, g, b) = match rgb {
        (0xE1, 0x24, 0x24) => (0xE1, 0x25, 0x25),
        (0x2A, 0x5B, 0xD8) => (0x2F, 0x5F, 0xD9),
        other => other,
    };
    [0xFF, r, g, b]
}

/// `PQTZ` beats (bar position 1-4, tempo x100, ms) as Engine grid markers:
/// one per tempo change, the first moved back to at or before the track
/// start and a closing one at or after its end, as Engine's own grids are.
/// Engine counts beats from 0 with the downbeat on multiples of 4.
fn engine_grid(beats: &[(u16, u16, u32)], rate: f64, sample_count: i64) -> Vec<GridMarker> {
    let Some(&(first_number, _, _)) = beats.first() else {
        return Vec::new();
    };
    let time = |i: usize| f64::from(beats[i].2) * rate / 1000.0;
    let first_beat_number = i64::from(first_number.clamp(1, 4)) - 1;
    let last = beats.len() - 1;
    let starts: Vec<usize> = (0..beats.len())
        .filter(|&i| i == 0 || beats[i].1 != beats[i - 1].1)
        .collect();

    let mut markers = Vec::with_capacity(starts.len() + 1);
    let mut intervals = Vec::with_capacity(starts.len());
    for (j, &start) in starts.iter().enumerate() {
        let end = starts.get(j + 1).copied().unwrap_or(last);
        let interval = if end > start {
            (time(end) - time(start)) / (end - start) as f64
        } else {
            60.0 * rate * 100.0 / f64::from(beats[start].1)
        };
        if !interval.is_finite() || interval <= 0.0 {
            return Vec::new();
        }
        intervals.push(interval);
        markers.push(GridMarker {
            sample_offset: time(start),
            beat_number: first_beat_number + start as i64,
            beats_until_next: (end - start) as i32,
        });
    }

    // Close past both the last beat and the end of the track.
    let interval = intervals[intervals.len() - 1];
    let tail = markers.last_mut().expect("at least one marker");
    let to_end = ((sample_count as f64 - tail.sample_offset) / interval).ceil() as i32;
    tail.beats_until_next = to_end.max(tail.beats_until_next + 1);
    let closing = GridMarker {
        sample_offset: tail.sample_offset + f64::from(tail.beats_until_next) * interval,
        beat_number: tail.beat_number + i64::from(tail.beats_until_next),
        beats_until_next: 0,
    };
    markers.push(closing);

    // Open at or before the track start.
    let head = &mut markers[0];
    if head.sample_offset > 0.0 {
        let back = (head.sample_offset / intervals[0]).ceil() as i32;
        head.sample_offset -= f64::from(back) * intervals[0];
        head.beat_number -= i64::from(back);
        head.beats_until_next += back;
    }
    markers
}

/// The `.EXT`'s `PWV4` colour preview (1200 entries; bytes 3/4/5 are the
/// low/mid/high bands, 0-127) as Engine's 1024-entry 0-255 overview.
fn overview_from_ext(ext: &[u8]) -> Option<Vec<[u8; 3]>> {
    if ext.get(0..4) != Some(b"PMAI") {
        return None;
    }
    let container = find_anlz_chunk_payload(ext, "PMAI").unwrap_or(ext);
    let payload = find_anlz_chunk_payload(container, "PWV4")?;
    let (entries, _) = payload.as_chunks::<6>();
    if entries.len() < OVERVIEW_ENTRIES / 4 {
        return None;
    }
    let n = entries.len();
    let overview: Vec<[u8; 3]> = (0..OVERVIEW_ENTRIES)
        .map(|i| {
            let start = i * n / OVERVIEW_ENTRIES;
            let end = ((i + 1) * n / OVERVIEW_ENTRIES).max(start + 1).min(n);
            let mut out = [0u8; 3];
            for entry in &entries[start..end] {
                for band in 0..3 {
                    let value = (u16::from(entry[3 + band].min(127)) * 2) as u8;
                    out[band] = out[band].max(value);
                }
            }
            out
        })
        .collect();
    overview.iter().any(|e| e != &[0; 3]).then_some(overview)
}

/// The PDB playlist tree as Engine lists: parents first, siblings in
/// rekordbox order, names made unique per parent (Engine requires it).
fn engine_playlists(pdb: &ParsedPdb, engine_ids: &HashMap<u32, i64>) -> Vec<EnginePlaylist> {
    let mut children: HashMap<u32, Vec<_>> = HashMap::new();
    for row in &pdb.playlist_tree {
        children.entry(row.parent_id).or_default().push(row);
    }
    for list in children.values_mut() {
        list.sort_by_key(|r| (r.sort_order, r.id));
    }
    let mut entries: HashMap<u32, Vec<_>> = HashMap::new();
    for entry in &pdb.playlist_entries {
        entries.entry(entry.playlist_id).or_default().push(entry);
    }

    let mut out = Vec::new();
    // (pdb id, engine parent id), depth-first so parents come first.
    let mut stack: Vec<(u32, i64)> = vec![(0, 0)];
    let mut visited = HashSet::new();
    while let Some((pdb_parent, engine_parent)) = stack.pop() {
        if !visited.insert(pdb_parent) {
            continue;
        }
        let mut used_titles = HashSet::new();
        let mut level = Vec::new();
        for row in children.get(&pdb_parent).into_iter().flatten() {
            if row.id == pdb_parent {
                continue;
            }
            let base = if row.name.trim().is_empty() {
                "Playlist".to_string()
            } else {
                row.name.clone()
            };
            let mut title = base.clone();
            let mut n = 2;
            while !used_titles.insert(title.clone()) {
                title = format!("{base} ({n})");
                n += 1;
            }
            let mut list_entries = entries.get(&row.id).cloned().unwrap_or_default();
            list_entries.sort_by_key(|e| e.entry_index);
            let id = out.len() as i64 + 1;
            out.push(EnginePlaylist {
                id,
                parent_id: engine_parent,
                title,
                track_ids: list_entries
                    .iter()
                    .filter_map(|e| engine_ids.get(&e.track_id).copied())
                    .collect(),
            });
            if row.row_is_folder {
                level.push((row.id, id));
            }
        }
        stack.extend(level.into_iter().rev());
    }
    out
}

/// One `AlbumArt` row per PDB artwork.
struct ArtCollector<'a> {
    usb_root: &'a Path,
    rows: Vec<EngineAlbumArt>,
    by_artwork: HashMap<u32, i64>,
}

impl<'a> ArtCollector<'a> {
    fn new(usb_root: &'a Path) -> Self {
        Self {
            usb_root,
            rows: vec![EngineAlbumArt {
                id: NO_ART_ID,
                ..EngineAlbumArt::default()
            }],
            by_artwork: HashMap::new(),
        }
    }

    fn id_for(&mut self, pdb: &ParsedPdb, artwork_id: u32) -> i64 {
        if artwork_id == 0 {
            return NO_ART_ID;
        }
        if let Some(id) = self.by_artwork.get(&artwork_id) {
            return *id;
        }
        let image = pdb
            .artworks
            .get(&artwork_id)
            .and_then(|path| load_art(self.usb_root, path));
        let id = match image {
            Some((hash, image)) => {
                let id = self.rows.len() as i64 + 1;
                self.rows.push(EngineAlbumArt {
                    id,
                    hash: Some(hash),
                    image: Some(image),
                });
                id
            }
            None => NO_ART_ID,
        };
        self.by_artwork.insert(artwork_id, id);
        id
    }

    fn into_rows(self) -> Vec<EngineAlbumArt> {
        self.rows
    }
}

/// The medium (`_m`) artwork when present, else the small one, as stored
/// (JPEG; the player decodes it), with its USB path as the hash.
fn load_art(usb_root: &Path, raw: &str) -> Option<(String, Vec<u8>)> {
    let small = resolve_usb_side_path(usb_root, raw)?;
    let medium = small.with_file_name(format!(
        "{}_m.{}",
        small.file_stem()?.to_string_lossy(),
        small.extension()?.to_string_lossy()
    ));
    let bytes = std::fs::read(&medium)
        .or_else(|_| std::fs::read(&small))
        .ok()?;
    image::guess_format(&bytes).ok()?;
    Some((raw.to_string(), bytes))
}

/// rekordbox's `YYYY-MM-DD` as Unix seconds (UTC midnight).
fn date_to_epoch(date: &str) -> Option<i64> {
    chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant_beats(first_number: u16, first_ms: u32, count: u32) -> Vec<(u16, u16, u32)> {
        // 120 BPM: a beat every 500 ms.
        (0..count)
            .map(|i| {
                (
                    ((u32::from(first_number) - 1 + i) % 4 + 1) as u16,
                    12000,
                    first_ms + i * 500,
                )
            })
            .collect()
    }

    #[test]
    fn constant_grid_spans_the_track() {
        let rate = 44100.0;
        let count = 44100 * 10; // 10 s
        let grid = engine_grid(&constant_beats(1, 250, 19), rate, count);
        assert_eq!(grid.len(), 2);
        let beat = 22050.0;
        // 250 ms = half a beat in: one beat back puts the head before 0.
        assert!((grid[0].sample_offset - (11025.0 - beat)).abs() < 1e-6);
        assert_eq!(grid[0].beat_number, -1);
        assert!(grid[1].sample_offset >= count as f64);
        assert_eq!(
            grid[1].beat_number - grid[0].beat_number,
            i64::from(grid[0].beats_until_next)
        );
        let spacing =
            (grid[1].sample_offset - grid[0].sample_offset) / f64::from(grid[0].beats_until_next);
        assert!((spacing - beat).abs() < 1e-6);
        // Beat 0 (a downbeat) lands on the first rekordbox beat.
        let zero = grid[0].sample_offset + f64::from(-grid[0].beat_number as i32) * spacing;
        assert!((zero - 11025.0).abs() < 1e-6);
    }

    #[test]
    fn grid_keeps_bar_position() {
        // First beat is beat 3 of its bar: Engine beat number 2.
        let grid = engine_grid(&constant_beats(3, 0, 8), 48000.0, 48000 * 4);
        assert_eq!(grid[0].sample_offset, 0.0);
        assert_eq!(grid[0].beat_number, 2);
    }

    #[test]
    fn tempo_change_adds_a_marker() {
        let mut beats = constant_beats(1, 0, 8);
        // From beat 8 on, 100 BPM (600 ms).
        let t8 = 4000;
        beats.extend((0..8).map(|i| (((8 + i) % 4 + 1) as u16, 10000, t8 + i * 600)));
        let grid = engine_grid(&beats, 1000.0, 20_000);
        assert_eq!(grid.len(), 3);
        assert_eq!(grid[0].beats_until_next, 8);
        assert_eq!(grid[1].sample_offset, 4000.0);
        assert_eq!(grid[1].beat_number, 8);
        let tail_spacing =
            (grid[2].sample_offset - grid[1].sample_offset) / f64::from(grid[1].beats_until_next);
        assert!((tail_spacing - 600.0).abs() < 1e-6);
        assert!(grid[2].sample_offset >= 20_000.0);
    }

    #[test]
    fn single_beat_uses_tempo() {
        let grid = engine_grid(&[(1, 12000, 1000)], 1000.0, 3000);
        assert_eq!(grid.len(), 2);
        assert_eq!(grid[0].sample_offset, 0.0);
        assert_eq!(grid[0].beat_number, -2);
        assert!(grid[1].sample_offset >= 3000.0);
    }

    #[test]
    fn empty_beats_give_no_grid() {
        assert!(engine_grid(&[], 44100.0, 1000).is_empty());
    }

    #[test]
    fn date_parses_to_utc_midnight() {
        assert_eq!(date_to_epoch("1970-01-02"), Some(86_400));
        assert_eq!(date_to_epoch("junk"), None);
    }

    #[test]
    fn cue_colours_use_device_palette() {
        let cue = |color_id| TrackCue {
            id: String::new(),
            position_ms: 0,
            color_id: Some(color_id),
            name: None,
            playback_start: false,
            memory: false,
        };
        assert_eq!(engine_cue_argb(&cue(1)), [0xFF, 0xDE, 0x44, 0xCF]);
        assert_eq!(engine_cue_argb(&cue(2)), [0xFF, 0xE1, 0x25, 0x25]);
        assert_eq!(engine_cue_argb(&cue(7)), [0xFF, 0x2F, 0x5F, 0xD9]);
        assert_eq!(engine_cue_argb(&cue(99)), [0xFF, 0x4E, 0xB6, 0x48]);
    }
}
