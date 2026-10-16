//! Genre, star-rating and release-date edits from the cue editor. Saved on
//! their own, so these changes leave the beat grid, key and cues exactly as
//! they are (no ANLZ rewrite, no BPM/key marked as user-edited).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::edb::{
    complete_partial_date, find_content_id_by_path, find_or_insert_genre, open_edb_rw,
    resolve_release_date,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    GetTracksMetadataSummaryRequest, GetUsbTracksMetadataSummaryRequest,
    SaveTrackMetadataEditsData, SaveTrackMetadataEditsRequest, SaveUsbTrackMetadataEditsData,
    SaveUsbTrackMetadataEditsRequest, SharedValue, TrackMetadataValues, TracksMetadataSummary,
    UsbMetadataEditResult, UsbMetadataEditTrack, WarningEntry,
};

use super::anlz::atomic_write_bytes;
use super::cues::{
    UsbLocalTrackHint, mark_track_edited_tx, pdb_page_size, pdb_track_by_media_path,
};
use super::export_helpers::{PdbLayoutProfile, load_table_columns_tx};
use super::usb_utils::resolve_usb_root;
use super::{BackendService, now};

/// Highest star rating (the PDB row byte and the eDB column are 0–5).
const MAX_RATING: u8 = 5;

/// A requested genre edit: `None` leaves the genre alone, `Some(None)` clears
/// it (an empty or blank name), `Some(Some(name))` sets the trimmed name.
type GenreEdit = Option<Option<String>>;

fn genre_edit(genre: Option<&str>) -> GenreEdit {
    genre.map(|g| Some(g.trim()).filter(|g| !g.is_empty()).map(str::to_string))
}

/// A requested release-date edit: `None` leaves the date alone, `Some(None)`
/// clears it, `Some(Some(date))` sets it (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`).
type ReleaseDateEdit = Option<Option<String>>;

/// Earliest year the editor accepts; the latest is next year.
const MIN_YEAR: u32 = 1000;

/// `None` leaves the date alone and blank text clears it. Anything else must
/// be `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (month and day may have one digit), a
/// real date in a year from 1000 to next year; it comes back zero-padded.
fn validate_release_date(text: Option<&str>) -> BackendResult<ReleaseDateEdit> {
    use chrono::Datelike;
    let Some(text) = text.map(str::trim) else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(Some(None));
    }
    let max_year = u32::try_from(chrono::Local::now().year() + 1).unwrap_or(u32::MAX);
    let invalid = || {
        BackendError::Validation(format!(
            "release date must be YYYY, YYYY-MM or YYYY-MM-DD, from {MIN_YEAR} to {max_year}"
        ))
    };
    let number = |part: &str, max_len: usize| {
        (!part.is_empty() && part.len() <= max_len && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
            .ok_or_else(invalid)
    };
    let parts: Vec<&str> = text.split('-').collect();
    let year = number(parts[0], 4)?;
    if parts[0].len() != 4 || !(MIN_YEAR..=max_year).contains(&year) {
        return Err(invalid());
    }
    let date = match parts[1..] {
        [] => format!("{year:04}"),
        [month] => match number(month, 2)? {
            month @ 1..=12 => format!("{year:04}-{month:02}"),
            _ => return Err(invalid()),
        },
        [month, day] => {
            chrono::NaiveDate::from_ymd_opt(year as i32, number(month, 2)?, number(day, 2)?)
                .ok_or_else(invalid)?
                .format("%Y-%m-%d")
                .to_string()
        }
        _ => return Err(invalid()),
    };
    Ok(Some(Some(date)))
}

/// The year of a validated release date.
fn year_of(date: &str) -> Option<u32> {
    date.get(..4)?.parse().ok()
}

/// The full date a release-date edit writes to a USB's PDB or eDB, which
/// hold only full dates: the edit itself, or for a year or month the date
/// already there when it falls inside it, else the start of that year or
/// month.
fn usb_release_date(edit: &str, existing: Option<&str>) -> String {
    if edit.len() == 10 {
        edit.to_string()
    } else {
        complete_partial_date(edit, existing.map(str::trim))
    }
}

fn validate_rating(rating: Option<u8>) -> BackendResult<Option<u8>> {
    match rating {
        Some(r) if r > MAX_RATING => Err(BackendError::Validation(format!(
            "rating must be 0 to {MAX_RATING}"
        ))),
        other => Ok(other),
    }
}

/// The library's genres plus `extra`, each once (ignoring case) and sorted
/// ignoring case: what the editor's genre field suggests.
pub(super) fn genre_options(
    conn: &Connection,
    extra: impl IntoIterator<Item = String>,
) -> BackendResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT TRIM(genre) FROM tracks WHERE genre IS NOT NULL AND TRIM(genre) != ''",
    )?;
    let library = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_key = std::collections::BTreeMap::<String, String>::new();
    for name in library.into_iter().chain(extra) {
        let name = name.trim();
        if !name.is_empty() {
            by_key
                .entry(name.to_lowercase())
                .or_insert_with(|| name.to_string());
        }
    }
    Ok(by_key.into_values().collect())
}

/// The fields the cue editor edits besides the analysis.
#[derive(Debug, Default, PartialEq)]
pub(super) struct EditableMetadata {
    pub genre: Option<String>,
    /// Star rating, 0 to 5.
    pub rating: u8,
    /// The release date export writes for the track (`YYYY-MM-DD`), or as
    /// much of one as the track has.
    pub release_date: Option<String>,
}

/// A library track's genre, star rating and the release date export writes
/// for it: the date the track has, completed from its file timestamp like
/// export does.
pub(super) fn library_metadata(
    conn: &Connection,
    track_id: &str,
) -> BackendResult<EditableMetadata> {
    let row = conn
        .query_row(
            "SELECT genre, rating, release_date, release_year,
                    MIN(COALESCE(file_created_at, file_modified_at),
                        COALESCE(file_modified_at, file_created_at))
             FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<u32>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?
        .unwrap_or_default();
    let genre = row
        .0
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    Ok(EditableMetadata {
        genre,
        rating: rating_byte(row.1),
        release_date: resolve_release_date(row.2.as_deref(), row.3.filter(|y| *y > 0), row.4),
    })
}

fn rating_byte(rating: Option<i64>) -> u8 {
    rating.unwrap_or(0).clamp(0, i64::from(MAX_RATING)) as u8
}

/// A USB track's genre, star rating and release date as its PDB row has
/// them, plus every genre name in that PDB.
pub(super) fn usb_metadata(
    usb_root: &Path,
    media_path: &str,
) -> BackendResult<(EditableMetadata, Vec<String>)> {
    let staged_pdb = super::usb_staging::stage_pdb(usb_root)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&std::fs::read(&staged_pdb)?)?;
    let row = pdb_track_by_media_path(&parsed, media_path)?;
    Ok((
        pdb_row_metadata(&parsed, row),
        parsed.genres.into_values().collect(),
    ))
}

/// A PDB track row's genre, rating and release date: the date string, else
/// just the year.
fn pdb_row_metadata(
    parsed: &crate::pdb_reader::ParsedPdb,
    row: &crate::pdb_reader::PdbTrackRow,
) -> EditableMetadata {
    EditableMetadata {
        genre: parsed
            .genres
            .get(&row.genre_id)
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty()),
        rating: row.rating.min(MAX_RATING),
        release_date: row
            .release_date
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .or_else(|| {
                row.release_year
                    .filter(|y| *y > 0)
                    .map(|y| format!("{y:04}"))
            }),
    }
}

/// Write a genre / rating / release-date edit onto a library track. The date
/// is stored as entered (export completes a year or month as it does a
/// tag's) with its year. The genre and the date are marked as the user's, so
/// a rescan or import keeps them over the file's tags.
fn apply_local_metadata_edits_tx(
    tx: &rusqlite::Transaction<'_>,
    track_id: &str,
    genre: &GenreEdit,
    rating: Option<u8>,
    release_date: &ReleaseDateEdit,
    now: &str,
) -> BackendResult<()> {
    if let Some(genre) = genre {
        tx.execute(
            "UPDATE tracks SET genre = ?1, genre_source = 'user' WHERE id = ?2",
            params![genre, track_id],
        )?;
    }
    if let Some(rating) = rating {
        tx.execute(
            "UPDATE tracks SET rating = ?1 WHERE id = ?2",
            params![rating, track_id],
        )?;
    }
    if let Some(date) = release_date {
        tx.execute(
            "UPDATE tracks SET release_date = ?1, release_year = ?2, release_date_source = 'user'
             WHERE id = ?3",
            params![date, date.as_deref().and_then(year_of), track_id],
        )?;
    }
    if genre.is_some() || rating.is_some() || release_date.is_some() {
        mark_track_edited_tx(tx, track_id, now)?;
    }
    Ok(())
}

/// A validated genre / rating / release-date edit: `None` fields are left
/// alone.
struct MetadataEdits {
    genre: GenreEdit,
    rating: Option<u8>,
    release_date: ReleaseDateEdit,
}

impl MetadataEdits {
    fn from_request(
        genre: Option<&str>,
        rating: Option<u8>,
        release_date: Option<&str>,
    ) -> BackendResult<Self> {
        Ok(Self {
            genre: genre_edit(genre),
            rating: validate_rating(rating)?,
            release_date: validate_release_date(release_date)?,
        })
    }
}

/// Trimmed, non-empty and each once, in request order.
fn distinct_trimmed<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .map(str::trim)
        .filter(|v| !v.is_empty() && seen.insert(v.to_string()))
        .map(str::to_string)
        .collect()
}

/// One field across tracks: what they all share, or mixed.
fn shared_value<T: PartialEq + Clone>(
    values: impl IntoIterator<Item = Option<T>>,
) -> SharedValue<T> {
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return SharedValue {
            value: None,
            mixed: false,
        };
    };
    if values.all(|v| v == first) {
        SharedValue {
            value: first,
            mixed: false,
        }
    } else {
        SharedValue {
            value: None,
            mixed: true,
        }
    }
}

fn summarize(metadata: &[EditableMetadata], genre_options: Vec<String>) -> TracksMetadataSummary {
    TracksMetadataSummary {
        count: metadata.len(),
        genre: shared_value(metadata.iter().map(|m| m.genre.clone())),
        rating: shared_value(metadata.iter().map(|m| Some(m.rating))),
        release_date: shared_value(metadata.iter().map(|m| m.release_date.clone())),
        genre_options,
    }
}

/// The staged PDB with each track row's genre, rating and release date (and
/// its year) patched in place, or `None` when the rows already have them. A
/// genre the PDB lacks gets one new genre row, shared by every track.
struct PdbMetadataPatch {
    /// The staged PDB and its patched bytes; `None` when nothing changed.
    write: Option<(PathBuf, Vec<u8>)>,
    /// Each track's release date after the edit, in `media_paths` order.
    release_dates: Vec<Option<String>>,
}

fn build_pdb_metadata_patch(
    usb_root: &Path,
    media_paths: &[String],
    edits: &MetadataEdits,
) -> BackendResult<PdbMetadataPatch> {
    let staged_pdb = super::usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&staged_pdb)?;
    let page_size = pdb_page_size(&bytes)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes)?;

    let genre_id = match &edits.genre {
        None => None,
        Some(None) => Some(0),
        Some(Some(name)) => {
            let wanted = name.to_lowercase();
            let existing = parsed
                .genres
                .iter()
                .filter(|(_, existing)| existing.trim().to_lowercase() == wanted)
                .map(|(id, _)| *id)
                .min();
            Some(match existing {
                Some(id) => id,
                None => {
                    let new_id = parsed.genres.keys().copied().max().unwrap_or(0) + 1;
                    crate::pdb_writer::append_genres_in_place(
                        &mut bytes,
                        &[(new_id, name.as_str())],
                        page_size,
                    )?;
                    new_id
                }
            })
        }
    };

    let mut mutations = Vec::new();
    let mut release_dates = Vec::with_capacity(media_paths.len());
    for media_path in media_paths {
        let row = pdb_track_by_media_path(&parsed, media_path)?;
        let mut data = super::repair::track_row_data_from_reader_row(row);
        let mut changed_fields = Vec::<&'static str>::new();
        if let Some(genre_id) = genre_id
            && row.genre_id != genre_id
        {
            data.genre_id = genre_id;
            changed_fields.push("genre_id");
        }
        if let Some(rating) = edits.rating
            && row.rating != rating
        {
            data.rating = Some(rating);
            changed_fields.push("rating");
        }
        if let Some(edit) = &edits.release_date {
            let date = edit
                .as_deref()
                .map(|edit| usb_release_date(edit, row.release_date.as_deref()));
            let year = date
                .as_deref()
                .and_then(year_of)
                .and_then(|y| u16::try_from(y).ok());
            if row.release_year != year {
                data.release_year = year;
                changed_fields.push("release_year");
            }
            if row.release_date.as_deref().unwrap_or("") != date.as_deref().unwrap_or("") {
                data.release_date = date.clone();
                changed_fields.push("release_date");
            }
            release_dates.push(date);
        } else {
            release_dates.push(pdb_row_metadata(&parsed, row).release_date);
        }
        if !changed_fields.is_empty() {
            mutations.push(crate::pdb_writer::PdbTrackRowMutation {
                row: data,
                changed_fields,
            });
        }
    }

    // No row changes: nothing is written, not even a new genre row.
    if mutations.is_empty() {
        return Ok(PdbMetadataPatch {
            write: None,
            release_dates,
        });
    }
    crate::pdb_writer::mutate_tracks_in_place(
        &mut bytes,
        &mutations,
        PdbLayoutProfile::DEFAULT,
        page_size,
    )?;
    Ok(PdbMetadataPatch {
        write: Some((staged_pdb, bytes)),
        release_dates,
    })
}

impl BackendService {
    pub fn save_track_metadata_edits(
        &self,
        req: SaveTrackMetadataEditsRequest,
    ) -> BackendResult<SaveTrackMetadataEditsData> {
        self.ensure_no_data_migration()?;
        let track_ids = distinct_trimmed(req.track_ids.iter().map(String::as_str));
        if track_ids.is_empty() {
            return Err(BackendError::Validation("trackIds is required".to_string()));
        }
        let edits = MetadataEdits::from_request(
            req.genre.as_deref(),
            req.rating,
            req.release_date.as_deref(),
        )?;

        let mut conn = self.db.connect()?;
        {
            let mut exists = conn.prepare("SELECT 1 FROM tracks WHERE id = ?1")?;
            for track_id in &track_ids {
                if !exists.exists(params![track_id])? {
                    return Err(BackendError::NotFound(format!(
                        "track not found: {track_id}"
                    )));
                }
            }
        }
        let tx = conn.transaction()?;
        let now = now();
        for track_id in &track_ids {
            apply_local_metadata_edits_tx(
                &tx,
                track_id,
                &edits.genre,
                edits.rating,
                &edits.release_date,
                &now,
            )?;
        }
        tx.commit()?;

        let items = track_ids
            .into_iter()
            .map(|track_id| {
                let metadata = library_metadata(&conn, &track_id)?;
                let year = conn
                    .query_row(
                        "SELECT release_year FROM tracks WHERE id = ?1",
                        params![track_id],
                        |row| row.get::<_, Option<u32>>(0),
                    )?
                    .filter(|y| *y > 0);
                Ok(TrackMetadataValues {
                    track_id,
                    genre: metadata.genre,
                    rating: metadata.rating,
                    year,
                    release_date: metadata.release_date,
                })
            })
            .collect::<BackendResult<Vec<_>>>()?;
        Ok(SaveTrackMetadataEditsData { items })
    }

    /// What library tracks share, for the selection bar's Edit.
    pub fn get_tracks_metadata_summary(
        &self,
        req: GetTracksMetadataSummaryRequest,
    ) -> BackendResult<TracksMetadataSummary> {
        let conn = self.db.connect()?;
        let metadata = distinct_trimmed(req.track_ids.iter().map(String::as_str))
            .iter()
            .map(|id| library_metadata(&conn, id))
            .collect::<BackendResult<Vec<_>>>()?;
        Ok(summarize(&metadata, genre_options(&conn, Vec::new())?))
    }

    /// What USB tracks share, as the USB's PDB has them.
    pub fn get_usb_tracks_metadata_summary(
        &self,
        req: GetUsbTracksMetadataSummaryRequest,
    ) -> BackendResult<TracksMetadataSummary> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let staged_pdb = super::usb_staging::stage_pdb(&usb_root)?;
        let parsed = crate::pdb_reader::parse_pdb_bytes(&std::fs::read(&staged_pdb)?)?;
        let metadata = distinct_trimmed(req.usb_media_paths_raw.iter().map(String::as_str))
            .iter()
            .map(|path| {
                Ok(pdb_row_metadata(
                    &parsed,
                    pdb_track_by_media_path(&parsed, path)?,
                ))
            })
            .collect::<BackendResult<Vec<_>>>()?;
        let usb_genres: Vec<String> = parsed.genres.into_values().collect();
        Ok(summarize(
            &metadata,
            genre_options(&*self.db.connect()?, usb_genres)?,
        ))
    }

    /// Write a genre / rating / release-date edit made from a USB view onto the USB's
    /// PDB and eDB in place, and onto each matched library track. Like the cue
    /// save, a track missing from either database blocks the save before
    /// anything is written. Holds `usb_write_lock` for the whole save.
    pub fn save_usb_track_metadata_edits_with_progress<F>(
        &self,
        req: SaveUsbTrackMetadataEditsRequest,
        mut on_progress: F,
    ) -> BackendResult<SaveUsbTrackMetadataEditsData>
    where
        F: FnMut(usize, usize, &str),
    {
        const STEPS: usize = 3;
        let _usb_write_guard = self
            .usb_write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        on_progress(0, STEPS, "USB: Checking tracks");

        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let mut seen = std::collections::HashSet::new();
        let tracks: Vec<UsbMetadataEditTrack> = req
            .tracks
            .into_iter()
            .map(|mut track| {
                track.usb_media_path_raw = track.usb_media_path_raw.trim().to_string();
                track
            })
            .filter(|t| {
                !t.usb_media_path_raw.is_empty() && seen.insert(t.usb_media_path_raw.clone())
            })
            .collect();
        if tracks.is_empty() {
            return Err(BackendError::Validation(
                "tracks with a usbMediaPathRaw are required".to_string(),
            ));
        }
        let edits = MetadataEdits::from_request(
            req.genre.as_deref(),
            req.rating,
            req.release_date.as_deref(),
        )?;

        let mut warnings: Vec<WarningEntry> = Vec::new();
        let mut edb_conn = open_edb_rw(&usb_root, &mut warnings).ok_or_else(|| {
            BackendError::NotFound(
                "USB library database (exportLibrary.db) not found or unreadable".to_string(),
            )
        })?;
        let content_ids = tracks
            .iter()
            .map(|t| {
                find_content_id_by_path(&edb_conn, &t.usb_media_path_raw)?.ok_or_else(|| {
                    BackendError::NotFound(format!(
                        "track not in this USB's library database: {}",
                        t.usb_media_path_raw
                    ))
                })
            })
            .collect::<BackendResult<Vec<_>>>()?;
        let media_paths: Vec<String> = tracks
            .iter()
            .map(|t| t.usb_media_path_raw.clone())
            .collect();
        let pdb_patch = build_pdb_metadata_patch(&usb_root, &media_paths, &edits)?;

        on_progress(1, STEPS, "USB: Writing export.pdb");
        if let Some((staged_pdb, bytes)) = pdb_patch.write {
            atomic_write_bytes(&staged_pdb, &bytes)?;
            super::usb_staging::write_back_if_changed(&usb_root, super::usb_staging::DbKind::Pdb)?;
        }

        // The library tracks, before the eDB commit, as the cue save does.
        let mut items = Vec::with_capacity(tracks.len());
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let now = now();
        for (track, release_date) in tracks.iter().zip(pdb_patch.release_dates) {
            let hint = UsbLocalTrackHint {
                usb_media_path_raw: &track.usb_media_path_raw,
                local_track_id: track.local_track_id.as_deref(),
                title: track.title.as_deref(),
                artist: track.artist.as_deref(),
                album: track.album.as_deref(),
                bpm: None,
            };
            let local_track_id = self.resolve_local_track_id_for_usb(&usb_root, &hint);
            match local_track_id.as_deref() {
                Some(track_id) => apply_local_metadata_edits_tx(
                    &tx,
                    track_id,
                    &edits.genre,
                    edits.rating,
                    &edits.release_date,
                    &now,
                )?,
                None => logging::emit(
                    Level::Warn,
                    "track-metadata.usb-save.no-local-match",
                    &format!(
                        "USB genre/rating/release date edit for {} matched no local track; device written, local master not",
                        track.usb_media_path_raw
                    ),
                ),
            }
            items.push(UsbMetadataEditResult {
                usb_media_path_raw: track.usb_media_path_raw.clone(),
                local_track_id,
                year: release_date.as_deref().and_then(year_of),
                release_date,
            });
        }
        tx.commit()?;

        on_progress(2, STEPS, "USB: Writing exportLibrary.db");
        let tx = edb_conn.transaction()?;
        let content_columns = load_table_columns_tx(&tx, "content")?;
        let edb_genre_id = match &edits.genre {
            Some(Some(name)) if content_columns.contains("genre_id") => {
                find_or_insert_genre(&tx, name)?
            }
            _ => None,
        };
        for content_id in &content_ids {
            if edits.genre.is_some() && content_columns.contains("genre_id") {
                tx.execute(
                    "UPDATE content SET genre_id = ?1 WHERE content_id = ?2",
                    params![edb_genre_id, content_id],
                )?;
            }
            if let Some(rating) = edits.rating
                && content_columns.contains("rating")
            {
                tx.execute(
                    "UPDATE content SET rating = ?1 WHERE content_id = ?2",
                    params![rating, content_id],
                )?;
            }
            if let Some(edit) = &edits.release_date {
                let date = match edit {
                    Some(edit) if content_columns.contains("releaseDate") => {
                        let existing: Option<String> = tx.query_row(
                            "SELECT releaseDate FROM content WHERE content_id = ?1",
                            params![content_id],
                            |row| row.get(0),
                        )?;
                        Some(usb_release_date(edit, existing.as_deref()))
                    }
                    Some(edit) => Some(edit.clone()),
                    None => None,
                };
                if content_columns.contains("releaseYear") {
                    tx.execute(
                        "UPDATE content SET releaseYear = ?1 WHERE content_id = ?2",
                        params![date.as_deref().and_then(year_of).unwrap_or(0), content_id],
                    )?;
                }
                if content_columns.contains("releaseDate") {
                    tx.execute(
                        "UPDATE content SET releaseDate = ?1 WHERE content_id = ?2",
                        params![date.unwrap_or_default(), content_id],
                    )?;
                }
            }
        }
        tx.commit()?;
        drop(edb_conn);
        super::usb_staging::write_back_if_changed(&usb_root, super::usb_staging::DbKind::Edb)?;
        self.refresh_engine_library(&usb_root);
        self.invalidate_usb_parse_cache();

        on_progress(STEPS, STEPS, "USB: Track details saved");
        Ok(SaveUsbTrackMetadataEditsData { items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_date_edit_leaves_clears_sets_or_rejects() {
        let set = |text: &str| validate_release_date(Some(text)).unwrap();
        assert_eq!(validate_release_date(None).unwrap(), None);
        assert_eq!(set(" "), Some(None));
        assert_eq!(set("1994"), Some(Some("1994".to_string())));
        assert_eq!(set("1994-3"), Some(Some("1994-03".to_string())));
        assert_eq!(set(" 1994-03-07 "), Some(Some("1994-03-07".to_string())));
        assert_eq!(set("2024-2-29"), Some(Some("2024-02-29".to_string())));
        for bad in [
            "999",
            "0999",
            "9999",
            "94",
            "1994-13",
            "1994-00",
            "2023-02-29",
            "1994-03-07-1",
            "1994/03/07",
            "1994-",
            "1994-003",
            "07.03.1994",
        ] {
            assert!(validate_release_date(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn usb_release_date_completes_a_year_or_month() {
        assert_eq!(
            usb_release_date("2016-05-03", Some("2009-10-09")),
            "2016-05-03"
        );
        assert_eq!(usb_release_date("2016", Some("2016-05-01")), "2016-05-01");
        assert_eq!(usb_release_date("2016", Some("2009-10-09")), "2016-01-01");
        assert_eq!(
            usb_release_date("2016-05", Some("2016-05-20")),
            "2016-05-20"
        );
        assert_eq!(
            usb_release_date("2016-05", Some("2016-06-20")),
            "2016-05-01"
        );
        assert_eq!(usb_release_date("2016", None), "2016-01-01");
    }
}
