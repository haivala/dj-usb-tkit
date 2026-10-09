//! Engine DJ library writer (`Engine Library/Database2/*.db`, plain SQLite).
//!
//! Engine OS players (Denon Prime, Numark Mixstream) read this library instead
//! of `export.pdb`. The schema is 3.0.2 exactly as a Mixstream Pro writes it
//! (`engine_schema/*.sql` is its `sqlite_master` DDL, verbatim). This module
//! only knows the format; `service::engine_export` gathers the data from a USB.
//! Format notes: `docs/ENGINE_DJ.md`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{BackendError, BackendResult};

pub const ENGINE_LIBRARY_DIR: &str = "Engine Library";
pub const ENGINE_DB_DIR: &str = "Database2";
pub const ENGINE_OVERVIEW_DIR: &str = "OverviewData";

const M_DB_SCHEMA: &str = include_str!("engine_schema/m.sql");
const HM_DB_SCHEMA: &str = include_str!("engine_schema/hm.sql");
const STM_DB_SCHEMA: &str = include_str!("engine_schema/stm.sql");

const SCHEMA_VERSION: (i64, i64, i64) = (3, 0, 2);
/// `PRAGMA user_version` of a device-written 3.0.2 library.
const USER_VERSION: i64 = 4_194_305;

pub const OVERVIEW_ENTRIES: usize = 1024;
pub const HOT_CUE_SLOTS: usize = 8;
const LOOP_SLOTS: usize = 8;

/// One library (`m.db`) to write.
#[derive(Debug, Clone, Default)]
pub struct EngineLibrary {
    pub db_uuid: String,
    /// `export.pdb` header sequence. The player imports the PDB again when it
    /// differs from `Information.lastRekordBoxLibraryImportReadCounter`.
    pub pdb_sequence: u32,
    pub current_played_indicator: Option<i64>,
    /// Lowest id a new track may get (ids of deleted tracks are never reused).
    pub track_id_floor: i64,
    pub tracks: Vec<EngineTrack>,
    /// Parents come before their children; siblings in display order.
    pub playlists: Vec<EnginePlaylist>,
    pub album_art: Vec<EngineAlbumArt>,
}

#[derive(Debug, Clone, Default)]
pub struct EngineAlbumArt {
    pub id: i64,
    pub hash: Option<String>,
    pub image: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct EngineTrack {
    pub id: i64,
    /// Relative to `Engine Library/`, e.g. `../Contents/A/B/x.mp3`.
    pub path: String,
    pub filename: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub label: Option<String>,
    pub composer: Option<String>,
    pub remixer: Option<String>,
    pub year: Option<i64>,
    pub length_seconds: Option<i64>,
    pub bitrate_kbps: Option<i64>,
    pub file_bytes: Option<i64>,
    pub file_type: Option<String>,
    pub bpm: Option<f64>,
    pub key: Option<u8>,
    /// 0..=100.
    pub rating: i64,
    pub date_added: Option<i64>,
    pub album_art_id: Option<i64>,
    pub sample_rate: Option<f64>,
    pub sample_count: Option<i64>,
    pub grid: Vec<GridMarker>,
    pub overview: Option<Vec<[u8; 3]>>,
    pub hot_cues: Vec<Option<EngineCue>>,
    pub main_cue: Option<f64>,
    /// Play state carried over from the previous `m.db`.
    pub played: Option<PlayedState>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayedState {
    pub is_played: Option<i64>,
    pub time_last_played: Option<i64>,
    pub played_indicator: Option<i64>,
}

impl EngineTrack {
    /// Analyzed when the player has a grid and an overview to show; otherwise
    /// it analyzes the track itself on load.
    pub fn is_analyzed(&self) -> bool {
        self.sample_rate.is_some() && !self.grid.is_empty() && self.overview.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridMarker {
    pub sample_offset: f64,
    pub beat_number: i64,
    pub beats_until_next: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineCue {
    pub label: String,
    pub sample_offset: f64,
    pub argb: [u8; 4],
}

#[derive(Debug, Clone, Default)]
pub struct EnginePlaylist {
    pub id: i64,
    /// 0 for a top-level list.
    pub parent_id: i64,
    pub title: String,
    pub track_ids: Vec<i64>,
}

/// What an existing `m.db` says that a rebuild keeps.
#[derive(Debug, Clone, Default)]
pub struct PreviousLibrary {
    pub db_uuid: String,
    pub current_played_indicator: Option<i64>,
    pub track_seq: i64,
    pub track_ids_by_path: HashMap<String, i64>,
    pub played_by_path: HashMap<String, PlayedState>,
}

pub fn engine_db_dir(usb_root: &Path) -> PathBuf {
    usb_root.join(ENGINE_LIBRARY_DIR).join(ENGINE_DB_DIR)
}

// -----------------------------------------------------------------------------
// Blobs
// -----------------------------------------------------------------------------

/// Qt's `qCompress`: u32 BE uncompressed length, then a zlib stream.
pub fn q_compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut encoder = ZlibEncoder::new(out, Compression::default());
    encoder
        .write_all(data)
        .expect("writing into a Vec cannot fail");
    encoder.finish().expect("writing into a Vec cannot fail")
}

/// `PerformanceData.trackData`.
pub fn encode_track_data(sample_rate: f64, sample_count: i64, key: Option<u8>) -> Vec<u8> {
    let mut raw = Vec::with_capacity(28);
    raw.extend_from_slice(&sample_rate.to_be_bytes());
    raw.extend_from_slice(&sample_count.to_be_bytes());
    // Average loudness 0..1; nothing we analyze maps onto it.
    raw.extend_from_slice(&0.5f64.to_be_bytes());
    raw.extend_from_slice(&i32::from(key.unwrap_or(0)).to_be_bytes());
    q_compress(&raw)
}

/// `PerformanceData.beatData`: the same grid as default and adjusted grid.
/// Header fields are big-endian, the markers little-endian.
pub fn encode_beat_data(sample_rate: f64, sample_count: i64, grid: &[GridMarker]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(17 + 2 * (8 + grid.len() * 24));
    raw.extend_from_slice(&sample_rate.to_be_bytes());
    raw.extend_from_slice(&(sample_count as f64).to_be_bytes());
    raw.push(1); // is beatgrid set
    for _ in 0..2 {
        raw.extend_from_slice(&(grid.len() as i64).to_be_bytes());
        for marker in grid {
            raw.extend_from_slice(&marker.sample_offset.to_le_bytes());
            raw.extend_from_slice(&marker.beat_number.to_le_bytes());
            raw.extend_from_slice(&marker.beats_until_next.to_le_bytes());
            raw.extend_from_slice(&0i32.to_le_bytes());
        }
    }
    q_compress(&raw)
}

/// `PerformanceData.overviewWaveFormData`: [`OVERVIEW_ENTRIES`] (low, mid,
/// high) entries, then the per-band maximum.
pub fn encode_overview(sample_count: i64, entries: &[[u8; 3]]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(16 + (entries.len() + 1) * 3);
    raw.extend_from_slice(&(entries.len() as i64).to_be_bytes());
    let per_entry = if entries.is_empty() {
        0.0
    } else {
        sample_count as f64 / entries.len() as f64
    };
    raw.extend_from_slice(&per_entry.to_be_bytes());
    let mut max = [0u8; 3];
    for entry in entries {
        raw.extend_from_slice(entry);
        for (band, value) in max.iter_mut().zip(entry) {
            *band = (*band).max(*value);
        }
    }
    raw.extend_from_slice(&max);
    q_compress(&raw)
}

/// `PerformanceData.quickCues`: [`HOT_CUE_SLOTS`] pads (unset = offset -1),
/// then the main cue.
pub fn encode_quick_cues(cues: &[Option<EngineCue>], main_cue: Option<f64>) -> Vec<u8> {
    let mut raw = Vec::with_capacity(160);
    raw.extend_from_slice(&(HOT_CUE_SLOTS as i64).to_be_bytes());
    for slot in 0..HOT_CUE_SLOTS {
        match cues.get(slot).and_then(Option::as_ref) {
            Some(cue) => {
                push_label(&mut raw, &cue.label);
                raw.extend_from_slice(&cue.sample_offset.to_be_bytes());
                raw.extend_from_slice(&cue.argb);
            }
            None => {
                raw.push(0);
                raw.extend_from_slice(&(-1.0f64).to_be_bytes());
                raw.extend_from_slice(&[0; 4]);
            }
        }
    }
    let main = main_cue.unwrap_or(-1.0);
    raw.extend_from_slice(&main.to_be_bytes()); // adjusted
    raw.push(0); // is adjusted
    raw.extend_from_slice(&main.to_be_bytes()); // default
    q_compress(&raw)
}

/// `PerformanceData.loops`: not compressed, little-endian. Always the empty
/// [`LOOP_SLOTS`] slots (the ANLZ we write carries no loops).
pub fn encode_empty_loops() -> Vec<u8> {
    let mut raw = Vec::with_capacity(8 + LOOP_SLOTS * 23);
    raw.extend_from_slice(&(LOOP_SLOTS as i64).to_le_bytes());
    for _ in 0..LOOP_SLOTS {
        raw.push(0); // label length
        raw.extend_from_slice(&(-1.0f64).to_le_bytes());
        raw.extend_from_slice(&(-1.0f64).to_le_bytes());
        raw.extend_from_slice(&[0, 0]); // start / end set
        raw.extend_from_slice(&[0; 4]); // colour
    }
    raw
}

/// A u8-length-prefixed UTF-8 label, cut at a char boundary to fit.
fn push_label(out: &mut Vec<u8>, label: &str) {
    let mut end = label.len().min(255);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    out.push(end as u8);
    out.extend_from_slice(&label.as_bytes()[..end]);
}

/// `Track.key`: 0 = C, 1 = Am, then on round the circle of fifths (major
/// even, relative minor odd). Takes a Camelot position (`8B` = C).
pub fn engine_key_from_camelot(number: u8, minor: bool) -> Option<u8> {
    (1..=12)
        .contains(&number)
        .then(|| (number + 4) % 12 * 2 + u8::from(minor))
}

// -----------------------------------------------------------------------------
// Database files
// -----------------------------------------------------------------------------

/// Read what a rebuild keeps from an existing `m.db`; `None` when there is
/// none or it isn't a 3.x library this writer understands.
pub fn read_previous_library(m_db: &Path) -> Option<PreviousLibrary> {
    if !m_db.is_file() {
        return None;
    }
    let conn =
        Connection::open_with_flags(m_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let (db_uuid, major, current_played_indicator): (String, i64, Option<i64>) = conn
        .query_row(
            "SELECT uuid, schemaVersionMajor, currentPlayedIndiciator FROM Information LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;
    if major != SCHEMA_VERSION.0 || db_uuid.is_empty() {
        return None;
    }
    let track_seq = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'Track'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()?
        .unwrap_or(0);
    let mut previous = PreviousLibrary {
        db_uuid,
        current_played_indicator,
        track_seq,
        ..PreviousLibrary::default()
    };
    let mut stmt = conn
        .prepare("SELECT id, path, isPlayed, timeLastPlayed, playedIndicator FROM Track")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                PlayedState {
                    is_played: row.get(2)?,
                    time_last_played: row.get(3)?,
                    played_indicator: row.get(4)?,
                },
            ))
        })
        .ok()?;
    for (id, path, played) in rows.flatten() {
        let Some(path) = path else { continue };
        previous.track_seq = previous.track_seq.max(id);
        previous.track_ids_by_path.insert(path.clone(), id);
        previous.played_by_path.insert(path, played);
    }
    Some(previous)
}

/// Write `library` as `<dir>/m.db`, replacing any existing one. The database
/// is built next to it and renamed into place, so a failed write leaves the old
/// library intact.
pub fn write_engine_m_db(dir: &Path, library: &EngineLibrary) -> BackendResult<()> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join("m.db");
    let tmp = dir.join("m.db.tmp");
    remove_db_files(&tmp)?;
    {
        let mut conn = create_db(&tmp, M_DB_SCHEMA)?;
        let tx = conn.transaction()?;
        insert_information(
            &tx,
            &library.db_uuid,
            library.current_played_indicator,
            Some(i64::from(library.pdb_sequence)),
        )?;
        insert_album_art(&tx, &library.album_art)?;
        insert_tracks(&tx, library)?;
        insert_playlists(&tx, library)?;
        tx.commit()?;
        conn.close().map_err(|(_, err)| BackendError::from(err))?;
    }
    // A leftover journal would be replayed onto the new file.
    remove_db_files(&target)?;
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// Create `hm.db`, `sm.db` and `stm.db` when missing or not schema 3.0.2,
/// leaving compatible ones (play history) alone, and the `OverviewData/<uuid>`
/// folders the player keeps per database.
pub fn ensure_aux_dbs(dir: &Path, m_db_uuid: &str) -> BackendResult<()> {
    std::fs::create_dir_all(dir)?;
    let mut overview_uuids = vec![m_db_uuid.to_string()];
    for (name, schema, overview) in [
        ("hm.db", HM_DB_SCHEMA, true),
        ("sm.db", STM_DB_SCHEMA, false),
        ("stm.db", STM_DB_SCHEMA, true),
    ] {
        let path = dir.join(name);
        let uuid = match compatible_db_uuid(&path) {
            Some(uuid) => uuid,
            None => {
                let uuid = uuid::Uuid::new_v4().to_string();
                remove_db_files(&path)?;
                let conn = create_db(&path, schema)?;
                insert_information(&conn, &uuid, None, None)?;
                conn.close().map_err(|(_, err)| BackendError::from(err))?;
                uuid
            }
        };
        if overview {
            overview_uuids.push(uuid);
        }
    }
    for uuid in overview_uuids {
        std::fs::create_dir_all(dir.join(ENGINE_OVERVIEW_DIR).join(uuid))?;
    }
    Ok(())
}

fn compatible_db_uuid(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let (uuid, version): (String, (i64, i64, i64)) = conn
        .query_row(
            "SELECT uuid, schemaVersionMajor, schemaVersionMinor, schemaVersionPatch
             FROM Information LIMIT 1",
            [],
            |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?))),
        )
        .ok()?;
    (version == SCHEMA_VERSION && !uuid.is_empty()).then_some(uuid)
}

fn remove_db_files(db: &Path) -> BackendResult<()> {
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let mut name = db.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::remove_file(PathBuf::from(name)) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.into()),
            _ => {}
        }
    }
    Ok(())
}

fn create_db(path: &Path, schema: &str) -> BackendResult<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!(
        "PRAGMA page_size = 4096; PRAGMA journal_mode = DELETE; PRAGMA user_version = {USER_VERSION};"
    ))?;
    conn.execute_batch(schema)?;
    Ok(conn)
}

fn insert_information(
    conn: &Connection,
    uuid: &str,
    current_played_indicator: Option<i64>,
    pdb_sequence: Option<i64>,
) -> BackendResult<()> {
    let indicator =
        current_played_indicator.unwrap_or_else(|| uuid::Uuid::new_v4().as_u64_pair().0 as i64);
    conn.execute(
        "INSERT INTO Information (id, uuid, schemaVersionMajor, schemaVersionMinor,
           schemaVersionPatch, currentPlayedIndiciator, lastRekordBoxLibraryImportReadCounter)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            uuid,
            SCHEMA_VERSION.0,
            SCHEMA_VERSION.1,
            SCHEMA_VERSION.2,
            indicator,
            pdb_sequence
        ],
    )?;
    Ok(())
}

fn insert_album_art(conn: &Connection, art: &[EngineAlbumArt]) -> BackendResult<()> {
    let mut stmt = conn.prepare("INSERT INTO AlbumArt (id, hash, albumArt) VALUES (?1, ?2, ?3)")?;
    let mut sorted: Vec<&EngineAlbumArt> = art.iter().collect();
    sorted.sort_by_key(|a| a.id);
    for a in sorted {
        stmt.execute(params![a.id, a.hash, a.image])?;
    }
    Ok(())
}

fn insert_tracks(conn: &Connection, library: &EngineLibrary) -> BackendResult<()> {
    let mut tracks: Vec<&EngineTrack> = library.tracks.iter().collect();
    // The schema rejects an id at or below the highest one so far.
    tracks.sort_by_key(|t| t.id);
    let mut insert = conn.prepare(
        "INSERT INTO Track (id, length, bpm, year, path, filename, bitrate, bpmAnalyzed,
           albumArtId, fileBytes, title, artist, album, genre, comment, label, composer,
           remixer, key, rating, timeLastPlayed, isPlayed, fileType, isAnalyzed, dateCreated,
           dateAdded, isAvailable, isMetadataOfPackedTrackChanged,
           isPerfomanceDataOfPackedTrackChanged, playedIndicator, isMetadataImported,
           pdbImportKey, isBeatGridLocked, originDatabaseUuid, originTrackId, streamingFlags,
           explicitLyrics)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
           ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?25, 1, 0, 0, ?26, 1, ?27, 0, ?28, ?1, 0, 0)",
    )?;
    let mut performance = conn.prepare(
        "UPDATE PerformanceData SET trackData = ?2, overviewWaveFormData = ?3, beatData = ?4,
           quickCues = ?5, loops = ?6, thirdPartySourceId = 1, activeOnLoadLoops = 0
         WHERE trackId = ?1",
    )?;
    let loops = encode_empty_loops();
    for t in tracks {
        let played = t.played.clone().unwrap_or_default();
        insert.execute(params![
            t.id,
            t.length_seconds,
            t.bpm.map(|b| b.round() as i64),
            t.year,
            t.path,
            t.filename,
            t.bitrate_kbps,
            t.bpm,
            t.album_art_id,
            t.file_bytes,
            t.title,
            t.artist,
            t.album,
            t.genre,
            t.comment,
            t.label,
            t.composer,
            t.remixer,
            t.key.map(i64::from),
            t.rating,
            played.time_last_played,
            played.is_played.unwrap_or(0),
            t.file_type,
            t.is_analyzed(),
            t.date_added,
            played.played_indicator,
            i64::from(library.pdb_sequence),
            library.db_uuid,
        ])?;
        let (track_data, overview, beat_data) = match (t.sample_rate, t.sample_count) {
            (Some(rate), Some(count)) if t.is_analyzed() => (
                Some(encode_track_data(rate, count, t.key)),
                t.overview.as_deref().map(|o| encode_overview(count, o)),
                Some(encode_beat_data(rate, count, &t.grid)),
            ),
            _ => (None, None, None),
        };
        performance.execute(params![
            t.id,
            track_data,
            overview,
            beat_data,
            encode_quick_cues(&t.hot_cues, t.main_cue),
            loops,
        ])?;
    }
    let max_id = library.tracks.iter().map(|t| t.id).max().unwrap_or(0);
    let seq = max_id.max(library.track_id_floor - 1);
    // Keep the previous library's high-water mark so a later track never gets
    // the id of one deleted here.
    if seq > 0
        && conn.execute(
            "UPDATE sqlite_sequence SET seq = ?1 WHERE name = 'Track'",
            params![seq],
        )? == 0
    {
        conn.execute(
            "INSERT INTO sqlite_sequence (name, seq) VALUES ('Track', ?1)",
            params![seq],
        )?;
    }
    Ok(())
}

fn insert_playlists(conn: &Connection, library: &EngineLibrary) -> BackendResult<()> {
    // Siblings form a linked list through `nextListId` (0 = last).
    let mut next_sibling: HashMap<i64, i64> = HashMap::new();
    let mut last_by_parent: HashMap<i64, i64> = HashMap::new();
    for list in &library.playlists {
        if let Some(prev) = last_by_parent.insert(list.parent_id, list.id) {
            next_sibling.insert(prev, list.id);
        }
    }
    let mut insert_list = conn.prepare(
        "INSERT INTO Playlist (id, title, parentListId, isPersisted, nextListId, lastEditTime,
           isExplicitlyExported)
         VALUES (?1, ?2, ?3, 1, ?4, strftime('%Y-%m-%d %H:%M:%S', 'now'), 1)",
    )?;
    let mut insert_entity = conn.prepare(
        "INSERT INTO PlaylistEntity (id, listId, trackId, databaseUuid, nextEntityId,
           membershipReference)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
    )?;
    let mut entity_id = 0i64;
    for list in &library.playlists {
        insert_list.execute(params![
            list.id,
            list.title,
            list.parent_id,
            next_sibling.get(&list.id).copied().unwrap_or(0)
        ])?;
        // A track appears once per list (`C_NAME_UNIQUE_FOR_LIST`).
        let mut seen = std::collections::HashSet::new();
        let tracks: Vec<i64> = list
            .track_ids
            .iter()
            .copied()
            .filter(|id| seen.insert(*id))
            .collect();
        let first = entity_id + 1;
        for (index, track_id) in tracks.iter().enumerate() {
            entity_id += 1;
            let next = if index + 1 < tracks.len() {
                first + index as i64 + 1
            } else {
                0
            };
            insert_entity.execute(params![entity_id, list.id, track_id, library.db_uuid, next])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn q_uncompress(blob: &[u8]) -> Vec<u8> {
        let len = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&blob[4..])
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out.len(), len);
        out
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    // An unset pad as a Mixstream Pro writes it: no label, offset -1.0, no colour.
    const EMPTY_PAD: &str = "00bff000000000000000000000";

    #[test]
    fn empty_quick_cues_match_device() {
        let raw = q_uncompress(&encode_quick_cues(&[], None));
        let expected = format!(
            "0000000000000008{}bff000000000000000bff0000000000000",
            EMPTY_PAD.repeat(8)
        );
        assert_eq!(raw.len(), 129);
        assert_eq!(hex(&raw), expected);
    }

    #[test]
    fn quick_cues_match_device_two_cue_track() {
        // "Dare to Jazz" on the Mixstream: pads 1 and 2, no main cue.
        let cue = |label: &str, offset: f64, argb: [u8; 4]| {
            Some(EngineCue {
                label: label.into(),
                sample_offset: offset,
                argb,
            })
        };
        let raw = q_uncompress(&encode_quick_cues(
            &[
                cue("Cue 1", 354035.0, [0xff, 0xde, 0x44, 0xcf]),
                cue("Cue 2", 1346594.0, [0xff, 0xe1, 0x25, 0x25]),
            ],
            None,
        ));
        let pad = |label: &str, offset: f64, argb: &str| {
            format!(
                "{:02x}{}{}{argb}",
                label.len(),
                hex(label.as_bytes()),
                hex(&offset.to_be_bytes())
            )
        };
        let expected = format!(
            "0000000000000008{}{}{}bff000000000000000bff0000000000000",
            pad("Cue 1", 354035.0, "ffde44cf"),
            pad("Cue 2", 1346594.0, "ffe12525"),
            EMPTY_PAD.repeat(6)
        );
        assert_eq!(hex(&raw), expected);
    }

    #[test]
    fn empty_loops_match_device() {
        let slot = "00000000000000f0bf000000000000f0bf000000000000";
        assert_eq!(
            hex(&encode_empty_loops()),
            format!("0800000000000000{}", slot.repeat(8))
        );
    }

    #[test]
    fn engine_key_follows_circle_of_fifths() {
        assert_eq!(engine_key_from_camelot(8, false), Some(0)); // C
        assert_eq!(engine_key_from_camelot(8, true), Some(1)); // Am
        assert_eq!(engine_key_from_camelot(9, false), Some(2)); // G
        assert_eq!(engine_key_from_camelot(7, true), Some(23)); // Dm
        assert_eq!(engine_key_from_camelot(7, false), Some(22)); // F
        assert_eq!(engine_key_from_camelot(1, true), Some(11)); // G#m
        assert_eq!(engine_key_from_camelot(13, true), None);
    }

    #[test]
    fn label_is_cut_at_char_boundary() {
        let mut out = Vec::new();
        push_label(&mut out, &"ä".repeat(200));
        assert_eq!(out[0], 254);
        assert!(std::str::from_utf8(&out[1..]).is_ok());
    }

    #[test]
    fn beat_data_layout() {
        let grid = [
            GridMarker {
                sample_offset: -100.0,
                beat_number: -1,
                beats_until_next: 10,
            },
            GridMarker {
                sample_offset: 5000.0,
                beat_number: 9,
                beats_until_next: 0,
            },
        ];
        let raw = q_uncompress(&encode_beat_data(44100.0, 6000, &grid));
        assert_eq!(raw.len(), 17 + 2 * (8 + 2 * 24));
        assert_eq!(&raw[..8], &44100.0f64.to_be_bytes());
        assert_eq!(&raw[8..16], &6000.0f64.to_be_bytes());
        assert_eq!(raw[16], 1);
        assert_eq!(&raw[17..25], &2i64.to_be_bytes());
        assert_eq!(&raw[25..33], &(-100.0f64).to_le_bytes());
        assert_eq!(&raw[33..41], &(-1i64).to_le_bytes());
        assert_eq!(&raw[41..45], &10i32.to_le_bytes());
        assert_eq!(&raw[73..81], &2i64.to_be_bytes());
    }

    fn sample_library() -> EngineLibrary {
        let track = |id: i64, path: &str| EngineTrack {
            id,
            path: path.into(),
            filename: path.rsplit('/').next().unwrap().into(),
            title: Some(format!("T{id}")),
            hot_cues: vec![None; HOT_CUE_SLOTS],
            ..EngineTrack::default()
        };
        EngineLibrary {
            db_uuid: "11111111-2222-3333-4444-555555555555".into(),
            pdb_sequence: 1476,
            track_id_floor: 1,
            tracks: vec![
                track(3, "../Contents/c.mp3"),
                track(1, "../Contents/a.mp3"),
                track(2, "../Contents/b.mp3"),
            ],
            playlists: vec![
                EnginePlaylist {
                    id: 1,
                    parent_id: 0,
                    title: "Folder".into(),
                    track_ids: vec![],
                },
                EnginePlaylist {
                    id: 2,
                    parent_id: 1,
                    title: "Inner".into(),
                    track_ids: vec![3, 1, 2, 3],
                },
                EnginePlaylist {
                    id: 3,
                    parent_id: 0,
                    title: "Second".into(),
                    track_ids: vec![2],
                },
                EnginePlaylist {
                    id: 4,
                    parent_id: 1,
                    title: "Inner 2".into(),
                    track_ids: vec![1, 2],
                },
            ],
            album_art: vec![EngineAlbumArt {
                id: 1,
                ..EngineAlbumArt::default()
            }],
            ..EngineLibrary::default()
        }
    }

    fn list_order(conn: &Connection, list_id: i64) -> Vec<i64> {
        // The first entity is the one no other entity points to.
        let mut order = Vec::new();
        let mut current: Option<i64> = conn
            .query_row(
                "SELECT id FROM PlaylistEntity e WHERE listId = ?1 AND id NOT IN
                   (SELECT nextEntityId FROM PlaylistEntity WHERE listId = ?1)",
                params![list_id],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        while let Some(id) = current {
            let (track, next): (i64, i64) = conn
                .query_row(
                    "SELECT trackId, nextEntityId FROM PlaylistEntity WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            order.push(track);
            current = (next != 0).then_some(next);
        }
        order
    }

    #[test]
    fn writes_library_with_linked_lists() {
        let dir = tempfile::tempdir().unwrap();
        let db_dir = dir.path().join("Database2");
        let library = sample_library();
        write_engine_m_db(&db_dir, &library).unwrap();
        ensure_aux_dbs(&db_dir, &library.db_uuid).unwrap();

        let conn = Connection::open(db_dir.join("m.db")).unwrap();
        let ok: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ok, "ok");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, USER_VERSION);
        let counter: i64 = conn
            .query_row(
                "SELECT lastRekordBoxLibraryImportReadCounter FROM Information",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(counter, 1476);

        // Siblings chain first-to-last through `nextListId`, as the device
        // writes them (its `PlaylistPath` view numbers them back to front).
        let siblings = |parent: i64| -> Vec<String> {
            let mut out = Vec::new();
            let mut current: Option<i64> = conn
                .query_row(
                    "SELECT id FROM Playlist WHERE parentListId = ?1 AND id NOT IN
                       (SELECT nextListId FROM Playlist WHERE parentListId = ?1)",
                    params![parent],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            while let Some(id) = current {
                let (title, next): (String, i64) = conn
                    .query_row(
                        "SELECT title, nextListId FROM Playlist WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                out.push(title);
                current = (next != 0).then_some(next);
            }
            out
        };
        assert_eq!(siblings(0), ["Folder", "Second"]);
        assert_eq!(siblings(1), ["Inner", "Inner 2"]);
        let path: String = conn
            .query_row("SELECT path FROM PlaylistPath WHERE id = 4", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(path, "Folder;Inner 2;");
        assert_eq!(list_order(&conn, 2), [3, 1, 2]);
        assert_eq!(list_order(&conn, 4), [1, 2]);
        assert_eq!(list_order(&conn, 3), [2]);

        let origin: (String, i64) = conn
            .query_row(
                "SELECT originDatabaseUuid, originTrackId FROM Track WHERE id = 3",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(origin, (library.db_uuid.clone(), 3));
        let perf: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM PerformanceData WHERE quickCues IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(perf, 3);

        for name in ["hm.db", "sm.db", "stm.db"] {
            assert!(compatible_db_uuid(&db_dir.join(name)).is_some(), "{name}");
        }
        assert!(
            db_dir
                .join(ENGINE_OVERVIEW_DIR)
                .join(&library.db_uuid)
                .is_dir()
        );
    }

    #[test]
    fn rebuild_keeps_ids_uuid_and_aux_dbs() {
        let dir = tempfile::tempdir().unwrap();
        let db_dir = dir.path().join("Database2");
        let mut library = sample_library();
        write_engine_m_db(&db_dir, &library).unwrap();
        ensure_aux_dbs(&db_dir, &library.db_uuid).unwrap();
        let hm_uuid = compatible_db_uuid(&db_dir.join("hm.db")).unwrap();
        // A stale journal next to the old file must not be replayed onto the new one.
        std::fs::write(db_dir.join("m.db-journal"), b"").unwrap();

        let previous = read_previous_library(&db_dir.join("m.db")).unwrap();
        assert_eq!(previous.db_uuid, library.db_uuid);
        assert_eq!(previous.track_seq, 3);
        assert_eq!(previous.track_ids_by_path["../Contents/b.mp3"], 2);

        // Drop track 3, add a new one: it gets id 4, not the freed 3.
        library.tracks.retain(|t| t.id != 3);
        library
            .playlists
            .iter_mut()
            .for_each(|p| p.track_ids.retain(|id| *id != 3));
        library.track_id_floor = previous.track_seq + 1;
        library.tracks.push(EngineTrack {
            id: 4,
            path: "../Contents/d.mp3".into(),
            filename: "d.mp3".into(),
            hot_cues: vec![None; HOT_CUE_SLOTS],
            ..EngineTrack::default()
        });
        write_engine_m_db(&db_dir, &library).unwrap();
        ensure_aux_dbs(&db_dir, &library.db_uuid).unwrap();

        assert!(!db_dir.join("m.db-journal").exists());
        assert_eq!(compatible_db_uuid(&db_dir.join("hm.db")).unwrap(), hm_uuid);
        let conn = Connection::open(db_dir.join("m.db")).unwrap();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM Track ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(ids, [1, 2, 4]);
        // The schema itself refuses a recycled id.
        let recycled = conn.execute(
            "INSERT INTO Track (id, path) VALUES (3, '../Contents/x.mp3')",
            [],
        );
        assert!(recycled.is_err());
    }
}
