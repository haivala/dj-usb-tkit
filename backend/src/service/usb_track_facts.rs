//! What the USB's two databases say about the tracks on it, merged per file.
//!
//! A stick can carry a track in the PDB only, the eDB only, or both. Every
//! "is this track already on the USB" question (export naming, file reuse,
//! the space estimate) reads `UsbTrackFacts`, so both databases always count
//! and the PDB-before-eDB precedence lives in one place.

use std::collections::{HashMap, HashSet};

use super::export_helpers::canonicalize_track_path_identity;
use super::track_matching::{ContentFingerprint, content_fingerprint_key, pdb_track_fingerprint};

/// (master_db_id, master_content_id, content_link, artwork_path) for a USB track.
pub(crate) type UsbTrackIdentity = (Option<u32>, Option<u32>, Option<u32>, Option<String>);

/// One track file the USB's databases point at.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct UsbTrackFact {
    /// The path as the database that first named it wrote it.
    pub path: String,
    /// Recorded file size: the PDB's as written, else the eDB's when positive.
    pub size: Option<u32>,
    /// `content_fingerprint_key` of the recorded size, title and artist.
    pub fingerprint: Option<ContentFingerprint>,
    /// The PDB row's IDs and artwork; `None` for a track only the eDB holds.
    pub identity: Option<UsbTrackIdentity>,
}

#[derive(Debug, Default)]
pub(crate) struct UsbTrackFacts {
    /// Keyed by `canonicalize_track_path_identity` (case-insensitive, as
    /// FAT32/exFAT are).
    pub by_path: HashMap<String, UsbTrackFact>,
    /// The one path a fingerprint points at; `None` when several paths share
    /// it (fingerprints are coarse, so an ambiguous one is never used). A
    /// fingerprint the PDB records keeps the PDB's path.
    pub path_by_fingerprint: HashMap<ContentFingerprint, Option<String>>,
    /// masterContentIds more than one PDB row carries: copied from another
    /// track by an older export, so not any one track's to keep.
    pub shared_master_content_ids: HashSet<u32>,
}

impl UsbTrackFacts {
    /// Both databases' tracks: the PDB's rows first, then the eDB's `content`
    /// rows filling in what the PDB doesn't know.
    pub(crate) fn read(
        parsed: Option<&crate::pdb_reader::ParsedPdb>,
        edb: Option<&rusqlite::Connection>,
    ) -> Self {
        let mut facts = Self::default();
        if let Some(parsed) = parsed {
            facts.add_pdb(parsed);
        }
        if let Some(conn) = edb {
            facts.add_edb(conn);
        }
        facts
    }

    /// The single USB path holding `fingerprint`, if exactly one does.
    pub(crate) fn path_for_fingerprint(&self, fingerprint: &ContentFingerprint) -> Option<&str> {
        self.path_by_fingerprint
            .get(fingerprint)
            .and_then(|path| path.as_deref())
    }

    /// The PDB identity of the row at `path`.
    pub(crate) fn identity_at(&self, path: &str) -> Option<&UsbTrackIdentity> {
        self.by_path
            .get(&canonicalize_track_path_identity(path))
            .and_then(|fact| fact.identity.as_ref())
    }

    fn add_pdb(&mut self, parsed: &crate::pdb_reader::ParsedPdb) {
        let mut seen_master_content_ids = HashSet::<u32>::new();
        for track in &parsed.tracks {
            if let Some(mci) = track.master_content_id
                && !seen_master_content_ids.insert(mci)
            {
                self.shared_master_content_ids.insert(mci);
            }
            let fingerprint = pdb_track_fingerprint(parsed, track);
            if let Some(fingerprint) = &fingerprint {
                record_existing_usb_path_by_fingerprint(
                    &mut self.path_by_fingerprint,
                    fingerprint.clone(),
                    &track.track_file_path,
                );
            }
            self.by_path.insert(
                canonicalize_track_path_identity(&track.track_file_path),
                UsbTrackFact {
                    path: track.track_file_path.clone(),
                    size: track.file_size_bytes,
                    fingerprint,
                    identity: Some((
                        track.master_db_id,
                        track.master_content_id,
                        track.content_link,
                        parsed.artworks.get(&track.artwork_id).cloned(),
                    )),
                },
            );
        }
    }

    fn add_edb(&mut self, conn: &rusqlite::Connection) {
        let pdb_fingerprints = self
            .path_by_fingerprint
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        for row in edb_track_rows(conn) {
            if let Some(fingerprint) = &row.fingerprint
                && !pdb_fingerprints.contains(fingerprint)
            {
                record_existing_usb_path_by_fingerprint(
                    &mut self.path_by_fingerprint,
                    fingerprint.clone(),
                    &row.path,
                );
            }
            let fact = self
                .by_path
                .entry(canonicalize_track_path_identity(&row.path))
                .or_insert_with(|| UsbTrackFact {
                    path: row.path,
                    ..UsbTrackFact::default()
                });
            if fact.size.is_none() {
                fact.size = row.size;
            }
            if fact.fingerprint.is_none() {
                fact.fingerprint = row.fingerprint;
            }
        }
    }
}

/// One eDB `content` row with what tells its track apart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdbContentTrack {
    pub content_id: Option<i64>,
    /// As written; may be blank.
    pub path: String,
    /// `fileSize` when positive.
    pub size: Option<u32>,
    /// `content_fingerprint_key` of `fileSize`, `title` and the artist's name.
    pub fingerprint: Option<ContentFingerprint>,
}

/// Every eDB `content` row as an `EdbContentTrack`. Columns a schema lacks
/// read as NULL; a schema without `path` gives no rows. A row that can't be
/// read is skipped; a query that can't run is an error.
pub(crate) fn read_edb_content_tracks(
    conn: &rusqlite::Connection,
) -> crate::error::BackendResult<Vec<EdbContentTrack>> {
    if !crate::edb::table_exists(conn, "content") {
        return Ok(Vec::new());
    }
    let columns = crate::edb::load_table_columns(conn, "content")?;
    let has = |name: &str| columns.iter().any(|column| column == name);
    if !has("path") {
        return Ok(Vec::new());
    }
    let column = |name: &str| {
        if has(name) {
            format!("c.{name}")
        } else {
            "NULL".to_string()
        }
    };
    let (artist, artist_join) =
        if has("artist_id_artist") && crate::edb::table_exists(conn, "artist") {
            (
                "ar.name",
                "LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist",
            )
        } else {
            ("NULL", "")
        };
    let sql = format!(
        "SELECT {}, c.path, CAST({} AS INTEGER), {}, {artist} FROM content c {artist_join}",
        column("content_id"),
        column("fileSize"),
        column("title"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    Ok(rows
        .flatten()
        .map(|(content_id, path, size, title, artist)| EdbContentTrack {
            content_id,
            path: path.unwrap_or_default(),
            fingerprint: content_fingerprint_key(
                size,
                title.as_deref().unwrap_or(""),
                artist.as_deref().unwrap_or(""),
            ),
            size: size
                .and_then(|size| u32::try_from(size).ok())
                .filter(|&size| size > 0),
        })
        .collect())
}

/// The eDB's tracks with a path, for `UsbTrackFacts`. A schema without
/// `path` (`FolderPath` + `FileNameL`) gives its paths alone
/// (`collect_edb_audio_paths`).
fn edb_track_rows(conn: &rusqlite::Connection) -> Vec<EdbContentTrack> {
    let has_path = crate::edb::load_table_columns(conn, "content")
        .is_ok_and(|columns| columns.iter().any(|column| column == "path"));
    let rows = if has_path {
        read_edb_content_tracks(conn).unwrap_or_default()
    } else {
        // Its schema warnings are the diagnostics' to report, not ours.
        let mut ignored_warnings = Vec::new();
        crate::service::diagnostics::collect_edb_audio_paths(conn, &mut ignored_warnings)
            .into_iter()
            .map(|path| EdbContentTrack {
                content_id: None,
                path,
                size: None,
                fingerprint: None,
            })
            .collect()
    };
    rows.into_iter()
        .filter(|row| !row.path.trim().is_empty())
        .collect()
}

/// Records `path` as the one USB path holding `fingerprint`; a second,
/// different path makes the fingerprint ambiguous (`None`) for good.
pub(crate) fn record_existing_usb_path_by_fingerprint(
    lookup: &mut HashMap<ContentFingerprint, Option<String>>,
    fingerprint: ContentFingerprint,
    path: &str,
) {
    let path_key = canonicalize_track_path_identity(path);
    lookup
        .entry(fingerprint)
        .and_modify(|existing| {
            if existing.as_deref().map(canonicalize_track_path_identity) != Some(path_key.clone()) {
                *existing = None;
            }
        })
        .or_insert_with(|| Some(path.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb_reader::{ParsedPdb, PdbTrackRow};

    fn edb_with(rows: &[(&str, Option<i64>, &str, Option<&str>)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE artist(artist_id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE content(content_id INTEGER PRIMARY KEY, path TEXT,
               fileSize INTEGER, title TEXT, artist_id_artist INTEGER);",
        )
        .unwrap();
        for (idx, (path, size, title, artist)) in rows.iter().enumerate() {
            let artist_id = artist.map(|name| {
                conn.execute("INSERT INTO artist(name) VALUES (?1)", [name])
                    .unwrap();
                conn.last_insert_rowid()
            });
            conn.execute(
                "INSERT INTO content(content_id, path, fileSize, title, artist_id_artist) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![idx as i64 + 1, path, size, title, artist_id],
            )
            .unwrap();
        }
        conn
    }

    fn pdb_with(rows: &[(&str, Option<u32>, &str, u32)]) -> ParsedPdb {
        let mut parsed = ParsedPdb::default();
        parsed.artists.insert(1, "Pdb Artist".to_string());
        for (path, size, title, mci) in rows {
            parsed.tracks.push(PdbTrackRow {
                track_file_path: path.to_string(),
                file_size_bytes: *size,
                title: title.to_string(),
                artist_id: 1,
                master_content_id: Some(*mci),
                ..PdbTrackRow::default()
            });
        }
        parsed
    }

    #[test]
    fn edb_only_track_counts_with_its_size_and_artist_fingerprint() {
        let edb = edb_with(&[(
            "/Contents/Edb/Only.mp3",
            Some(900),
            "Only",
            Some("Edb Artist"),
        )]);
        let facts = UsbTrackFacts::read(None, Some(&edb));
        let fingerprint = content_fingerprint_key(Some(900), "Only", "Edb Artist").unwrap();
        let fact = &facts.by_path["/contents/edb/only.mp3"];
        assert_eq!(fact.size, Some(900));
        assert_eq!(fact.fingerprint.as_ref(), Some(&fingerprint));
        assert_eq!(fact.identity, None);
        assert_eq!(
            facts.path_for_fingerprint(&fingerprint),
            Some("/Contents/Edb/Only.mp3"),
            "the artist join is part of the fingerprint"
        );
    }

    #[test]
    fn pdb_row_wins_and_edb_fills_what_it_lacks() {
        let parsed = pdb_with(&[("/Contents/Both.mp3", None, "Both", 7)]);
        let edb = edb_with(&[("/contents/BOTH.mp3", Some(500), "Both", None)]);
        let facts = UsbTrackFacts::read(Some(&parsed), Some(&edb));
        assert_eq!(facts.by_path.len(), 1);
        let fact = &facts.by_path["/contents/both.mp3"];
        assert_eq!(fact.path, "/Contents/Both.mp3");
        assert_eq!(fact.size, Some(500), "size from the eDB");
        assert_eq!(fact.identity.as_ref().and_then(|id| id.1), Some(7));
    }

    #[test]
    fn pdb_fingerprint_keeps_the_pdb_path() {
        let parsed = pdb_with(&[("/Contents/Pdb.mp3", Some(500), "Same", 1)]);
        let edb = edb_with(&[("/Contents/Edb.mp3", Some(500), "Same", Some("Pdb Artist"))]);
        let facts = UsbTrackFacts::read(Some(&parsed), Some(&edb));
        let fingerprint = content_fingerprint_key(Some(500), "Same", "Pdb Artist").unwrap();
        assert_eq!(
            facts.path_for_fingerprint(&fingerprint),
            Some("/Contents/Pdb.mp3")
        );
        assert!(
            facts.by_path.contains_key("/contents/edb.mp3"),
            "name taken"
        );
    }

    #[test]
    fn duplicate_fingerprints_are_ambiguous_and_shared_ids_flagged() {
        let parsed = pdb_with(&[
            ("/Contents/A.mp3", Some(500), "Same", 3),
            ("/Contents/B.mp3", Some(500), "Same", 3),
        ]);
        let facts = UsbTrackFacts::read(Some(&parsed), None);
        let fingerprint = content_fingerprint_key(Some(500), "Same", "Pdb Artist").unwrap();
        assert_eq!(facts.path_for_fingerprint(&fingerprint), None);
        assert!(facts.shared_master_content_ids.contains(&3));
    }

    #[test]
    fn edb_without_size_or_title_columns_still_takes_its_names() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE content(content_id INTEGER PRIMARY KEY, path TEXT);
             INSERT INTO content VALUES (1, '/Contents/Bare.mp3');",
        )
        .unwrap();
        let facts = UsbTrackFacts::read(None, Some(&conn));
        let fact = &facts.by_path["/contents/bare.mp3"];
        assert_eq!((fact.size, fact.fingerprint.as_ref()), (None, None));
    }

    #[test]
    fn duplicate_fingerprint_lookup_stays_ambiguous() {
        let fingerprint = content_fingerprint_key(Some(1234), "Title", "Artist").unwrap();
        let mut lookup = HashMap::<ContentFingerprint, Option<String>>::new();
        record_existing_usb_path_by_fingerprint(
            &mut lookup,
            fingerprint.clone(),
            "/Contents/A.mp3",
        );
        assert_eq!(lookup[&fingerprint].as_deref(), Some("/Contents/A.mp3"));
        record_existing_usb_path_by_fingerprint(
            &mut lookup,
            fingerprint.clone(),
            "/Contents/B.mp3",
        );
        assert_eq!(lookup[&fingerprint], None);
        record_existing_usb_path_by_fingerprint(
            &mut lookup,
            fingerprint.clone(),
            "/Contents/A.mp3",
        );
        assert_eq!(
            lookup[&fingerprint], None,
            "ambiguous fingerprints must stay disabled rather than flipping between paths"
        );
    }
}
