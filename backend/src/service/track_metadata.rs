//! Genre and star-rating edits from the cue editor. Saved on their own, so a
//! genre or rating change leaves the beat grid, key and cues exactly as they
//! are (no ANLZ rewrite, no BPM/key marked as user-edited).

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

/// A library track's genre and star rating.
pub(super) fn library_genre_rating(
    conn: &Connection,
    track_id: &str,
) -> BackendResult<(Option<String>, u8)> {
    let row = conn
        .query_row(
            "SELECT genre, rating FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                ))
            },
        )
        .optional()?
        .unwrap_or_default();
    let genre = row
        .0
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    Ok((genre, rating_byte(row.1)))
}

fn rating_byte(rating: Option<i64>) -> u8 {
    rating.unwrap_or(0).clamp(0, i64::from(MAX_RATING)) as u8
}

/// A USB track's genre and star rating as its PDB row has them, plus every
/// genre name in that PDB.
pub(super) fn usb_genre_rating(
    usb_root: &Path,
    media_path: &str,
) -> BackendResult<(Option<String>, u8, Vec<String>)> {
    let staged_pdb = super::usb_staging::stage_pdb(usb_root)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&std::fs::read(&staged_pdb)?)?;
    let row = pdb_track_by_media_path(&parsed, media_path)?;
    let genre = parsed
        .genres
        .get(&row.genre_id)
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    let rating = row.rating.min(MAX_RATING);
    Ok((genre, rating, parsed.genres.into_values().collect()))
}

/// Write a genre / rating edit onto a library track. The genre is marked as
/// the user's, so a rescan or import keeps it over the file's tag.
fn apply_local_metadata_edits_tx(
    tx: &rusqlite::Transaction<'_>,
    track_id: &str,
    genre: &GenreEdit,
    rating: Option<u8>,
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
    if genre.is_some() || rating.is_some() {
        mark_track_edited_tx(tx, track_id, now)?;
    }
    Ok(())
}

/// The staged PDB with the track row's genre and rating patched in place, or
/// `None` when the row already has them. A genre the PDB lacks gets a new
/// genre row.
fn build_pdb_metadata_patch(
    usb_root: &Path,
    media_path: &str,
    genre: &GenreEdit,
    rating: Option<u8>,
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
        apply_local_metadata_edits_tx(&tx, &track_id, &genre, rating, &now())?;
        tx.commit()?;

        let (genre, rating) = library_genre_rating(&conn, &track_id)?;
        Ok(SaveTrackMetadataEditsData {
            track_id,
            genre,
            rating,
        })
    }

    pub fn save_usb_track_metadata_edits(
        &self,
        req: SaveUsbTrackMetadataEditsRequest,
    ) -> BackendResult<SaveUsbTrackMetadataEditsData> {
        self.save_usb_track_metadata_edits_with_progress(req, |_, _, _| {})
    }

    /// Write a genre / rating edit made from a USB view onto the USB's PDB and
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
        let pdb_patch = build_pdb_metadata_patch(&usb_root, media_path, &genre, rating)?;

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
                apply_local_metadata_edits_tx(&tx, track_id, &genre, rating, &now())?;
                tx.commit()?;
            }
            None => logging::emit(
                Level::Warn,
                "track-metadata.usb-save.no-local-match",
                &format!(
                    "USB genre/rating edit for {media_path} matched no local track; device written, local master not"
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
