//! Genre, star-rating and release-year edits from the cue editor. Saved on
//! their own, so these changes leave the beat grid, key and cues exactly as
//! they are (no ANLZ rewrite, no BPM/key marked as user-edited).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::edb::{find_content_id_by_path, find_or_insert_genre, open_edb_rw};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    SaveTrackMetadataEditsData, SaveTrackMetadataEditsRequest, SaveUsbTrackMetadataEditsData,
    SaveUsbTrackMetadataEditsRequest, WarningEntry,
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

/// A requested year edit: `None` leaves the year alone, `Some(None)` clears
/// it, `Some(Some(year))` sets it.
type YearEdit = Option<Option<u32>>;

/// Earliest year the editor accepts; the latest is next year.
const MIN_YEAR: u32 = 1000;

/// `None` leaves the year alone, 0 clears it, anything else must be a year
/// from 1000 to next year.
fn validate_year(year: Option<u32>) -> BackendResult<YearEdit> {
    use chrono::Datelike;
    let max_year = u32::try_from(chrono::Local::now().year() + 1).unwrap_or(u32::MAX);
    match year {
        None => Ok(None),
        Some(0) => Ok(Some(None)),
        Some(y) if (MIN_YEAR..=max_year).contains(&y) => Ok(Some(Some(y))),
        Some(_) => Err(BackendError::Validation(format!(
            "year must be {MIN_YEAR} to {max_year}"
        ))),
    }
}

/// The release date to store with a year the user set: a date from another
/// year becomes 1 January of that year, as export does with a file date that
/// contradicts the year tag. `None` when `date` already fits (or is empty).
fn release_date_for_user_year(date: Option<&str>, year: u32) -> Option<String> {
    let date = date.map(str::trim).filter(|d| !d.is_empty())?;
    (!date.starts_with(&format!("{year:04}"))).then(|| format!("{year:04}-01-01"))
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
    pub year: Option<u32>,
}

/// A library track's genre, star rating and release year.
pub(super) fn library_metadata(
    conn: &Connection,
    track_id: &str,
) -> BackendResult<EditableMetadata> {
    let row = conn
        .query_row(
            "SELECT genre, rating, release_year FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<u32>>(2)?,
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
        year: row.2.filter(|y| *y > 0),
    })
}

fn rating_byte(rating: Option<i64>) -> u8 {
    rating.unwrap_or(0).clamp(0, i64::from(MAX_RATING)) as u8
}

/// A USB track's genre, star rating and release year as its PDB row has
/// them, plus every genre name in that PDB.
pub(super) fn usb_metadata(
    usb_root: &Path,
    media_path: &str,
) -> BackendResult<(EditableMetadata, Vec<String>)> {
    let staged_pdb = super::usb_staging::stage_pdb(usb_root)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&std::fs::read(&staged_pdb)?)?;
    let row = pdb_track_by_media_path(&parsed, media_path)?;
    let genre = parsed
        .genres
        .get(&row.genre_id)
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    let metadata = EditableMetadata {
        genre,
        rating: row.rating.min(MAX_RATING),
        year: row.release_year.map(u32::from),
    };
    Ok((metadata, parsed.genres.into_values().collect()))
}

/// Write a genre / rating / year edit onto a library track. The genre and
/// year are marked as the user's, so a rescan or import keeps them over the
/// file's tags.
fn apply_local_metadata_edits_tx(
    tx: &rusqlite::Transaction<'_>,
    track_id: &str,
    genre: &GenreEdit,
    rating: Option<u8>,
    year: YearEdit,
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
    if let Some(year) = year {
        tx.execute(
            "UPDATE tracks SET release_year = ?1, release_year_source = 'user' WHERE id = ?2",
            params![year, track_id],
        )?;
    }
    if genre.is_some() || rating.is_some() || year.is_some() {
        mark_track_edited_tx(tx, track_id, now)?;
    }
    Ok(())
}

/// The staged PDB with the track row's genre, rating and year patched in
/// place, or `None` when the row already has them. A genre the PDB lacks gets
/// a new genre row.
fn build_pdb_metadata_patch(
    usb_root: &Path,
    media_path: &str,
    genre: &GenreEdit,
    rating: Option<u8>,
    year: YearEdit,
) -> BackendResult<Option<(PathBuf, Vec<u8>)>> {
    let staged_pdb = super::usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&staged_pdb)?;
    let page_size = pdb_page_size(&bytes)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes)?;
    let row = pdb_track_by_media_path(&parsed, media_path)?;
    let mut data = super::repair::track_row_data_from_reader_row(row);
    let mut changed_fields = Vec::<&'static str>::new();

    if let Some(genre) = genre {
        let genre_id = match genre.as_deref() {
            None => 0,
            Some(name) => {
                let wanted = name.to_lowercase();
                match parsed
                    .genres
                    .iter()
                    .filter(|(_, existing)| existing.trim().to_lowercase() == wanted)
                    .map(|(id, _)| *id)
                    .min()
                {
                    Some(id) => id,
                    None => {
                        let new_id = parsed.genres.keys().copied().max().unwrap_or(0) + 1;
                        crate::pdb_writer::append_genres_in_place(
                            &mut bytes,
                            &[(new_id, name)],
                            page_size,
                        )?;
                        new_id
                    }
                }
            }
        };
        if row.genre_id != genre_id {
            data.genre_id = genre_id;
            changed_fields.push("genre_id");
        }
    }

    if let Some(rating) = rating
        && row.rating != rating
    {
        data.rating = Some(rating);
        changed_fields.push("rating");
    }

    if let Some(year) = year {
        let wanted = year.and_then(|y| u16::try_from(y).ok());
        if row.release_year != wanted {
            data.release_year = wanted;
            changed_fields.push("release_year");
        }
        if let Some(date) =
            year.and_then(|y| release_date_for_user_year(row.release_date.as_deref(), y))
        {
            data.release_date = Some(date);
            changed_fields.push("release_date");
        }
    }

    if changed_fields.is_empty() {
        return Ok(None);
    }
    crate::pdb_writer::mutate_tracks_in_place(
        &mut bytes,
        &[crate::pdb_writer::PdbTrackRowMutation {
            row: data,
            changed_fields,
        }],
        PdbLayoutProfile::DEFAULT,
        page_size,
    )?;
    Ok(Some((staged_pdb, bytes)))
}

impl BackendService {
    pub fn save_track_metadata_edits(
        &self,
        req: SaveTrackMetadataEditsRequest,
    ) -> BackendResult<SaveTrackMetadataEditsData> {
        self.ensure_no_data_migration()?;
        let track_id = req.track_id.trim().to_string();
        if track_id.is_empty() {
            return Err(BackendError::Validation("trackId is required".to_string()));
        }
        let genre = genre_edit(req.genre.as_deref());
        let rating = validate_rating(req.rating)?;
        let year = validate_year(req.year)?;

        let mut conn = self.db.connect()?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM tracks WHERE id = ?1",
                params![track_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(BackendError::NotFound(format!(
                "track not found: {track_id}"
            )));
        }
        let tx = conn.transaction()?;
        apply_local_metadata_edits_tx(&tx, &track_id, &genre, rating, year, &now())?;
        tx.commit()?;

        let metadata = library_metadata(&conn, &track_id)?;
        Ok(SaveTrackMetadataEditsData {
            track_id,
            genre: metadata.genre,
            rating: metadata.rating,
            year: metadata.year,
        })
    }

    pub fn save_usb_track_metadata_edits(
        &self,
        req: SaveUsbTrackMetadataEditsRequest,
    ) -> BackendResult<SaveUsbTrackMetadataEditsData> {
        self.save_usb_track_metadata_edits_with_progress(req, |_, _, _| {})
    }

    /// Write a genre / rating / year edit made from a USB view onto the USB's PDB and
    /// eDB in place, and onto the matched library track. Like the cue save, a
    /// track missing from either database blocks the save before anything is
    /// written. Holds `usb_write_lock` for the whole save.
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
        on_progress(0, STEPS, "USB: Checking track");

        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let media_path = req.usb_media_path_raw.trim();
        if media_path.is_empty() {
            return Err(BackendError::Validation(
                "usbMediaPathRaw is required".to_string(),
            ));
        }
        let genre = genre_edit(req.genre.as_deref());
        let rating = validate_rating(req.rating)?;
        let year = validate_year(req.year)?;

        let mut warnings: Vec<WarningEntry> = Vec::new();
        let mut edb_conn = open_edb_rw(&usb_root, &mut warnings).ok_or_else(|| {
            BackendError::NotFound(
                "USB library database (exportLibrary.db) not found or unreadable".to_string(),
            )
        })?;
        let content_id = find_content_id_by_path(&edb_conn, media_path)?.ok_or_else(|| {
            BackendError::NotFound(format!(
                "track not in this USB's library database: {media_path}"
            ))
        })?;
        let pdb_patch = build_pdb_metadata_patch(&usb_root, media_path, &genre, rating, year)?;

        on_progress(1, STEPS, "USB: Writing export.pdb");
        if let Some((staged_pdb, bytes)) = pdb_patch {
            atomic_write_bytes(&staged_pdb, &bytes)?;
            super::usb_staging::write_back_if_changed(&usb_root, super::usb_staging::DbKind::Pdb)?;
        }

        // The library track, before the eDB commit, as the cue save does.
        let hint = UsbLocalTrackHint {
            usb_media_path_raw: media_path,
            local_track_id: req.local_track_id.as_deref(),
            title: req.title.as_deref(),
            artist: req.artist.as_deref(),
            album: req.album.as_deref(),
            bpm: None,
        };
        let local_track_id = self.resolve_local_track_id_for_usb(&usb_root, &hint);
        match local_track_id.as_deref() {
            Some(track_id) => {
                let mut conn = self.db.connect()?;
                let tx = conn.transaction()?;
                apply_local_metadata_edits_tx(&tx, track_id, &genre, rating, year, &now())?;
                tx.commit()?;
            }
            None => logging::emit(
                Level::Warn,
                "track-metadata.usb-save.no-local-match",
                &format!(
                    "USB genre/rating/year edit for {media_path} matched no local track; device written, local master not"
                ),
            ),
        }

        on_progress(2, STEPS, "USB: Writing exportLibrary.db");
        let tx = edb_conn.transaction()?;
        let content_columns = load_table_columns_tx(&tx, "content")?;
        if let Some(genre) = &genre
            && content_columns.contains("genre_id")
        {
            let genre_id = match genre.as_deref() {
                Some(name) => find_or_insert_genre(&tx, name)?,
                None => None,
            };
            tx.execute(
                "UPDATE content SET genre_id = ?1 WHERE content_id = ?2",
                params![genre_id, content_id],
            )?;
        }
        if let Some(rating) = rating
            && content_columns.contains("rating")
        {
            tx.execute(
                "UPDATE content SET rating = ?1 WHERE content_id = ?2",
                params![rating, content_id],
            )?;
        }
        if let Some(year) = year {
            if content_columns.contains("releaseYear") {
                tx.execute(
                    "UPDATE content SET releaseYear = ?1 WHERE content_id = ?2",
                    params![year.unwrap_or(0), content_id],
                )?;
            }
            if let Some(year) = year
                && content_columns.contains("releaseDate")
            {
                let date: Option<String> = tx.query_row(
                    "SELECT releaseDate FROM content WHERE content_id = ?1",
                    params![content_id],
                    |row| row.get(0),
                )?;
                if let Some(date) = release_date_for_user_year(date.as_deref(), year) {
                    tx.execute(
                        "UPDATE content SET releaseDate = ?1 WHERE content_id = ?2",
                        params![date, content_id],
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
        Ok(SaveUsbTrackMetadataEditsData {
            local_updated: local_track_id.is_some(),
            local_track_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_edit_leaves_clears_sets_or_rejects() {
        assert_eq!(validate_year(None).unwrap(), None);
        assert_eq!(validate_year(Some(0)).unwrap(), Some(None));
        assert_eq!(validate_year(Some(1994)).unwrap(), Some(Some(1994)));
        assert!(validate_year(Some(999)).is_err());
        assert!(validate_year(Some(9999)).is_err());
    }

    #[test]
    fn release_date_follows_a_user_year_it_contradicts() {
        assert_eq!(
            release_date_for_user_year(Some("2009-10-09"), 2016).as_deref(),
            Some("2016-01-01")
        );
        assert_eq!(release_date_for_user_year(Some("2016-05-01"), 2016), None);
        assert_eq!(release_date_for_user_year(Some(" "), 2016), None);
        assert_eq!(release_date_for_user_year(None, 2016), None);
    }
}
