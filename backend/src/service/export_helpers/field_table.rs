//! The track field table: which manifest field, eDB `content` column and
//! PDB track field carry the same value, for both export and the strict
//! parity repair, and which fields are deliberately not synced.
//!
//! Guards keep it complete: [`ExportManifestTrack`] and [`PdbTrackRowData`]
//! are destructured here with no `..`, so a new field doesn't compile until
//! it is mapped or listed as not synced, and a test fails on any eDB
//! template `content` column in neither list below.

use std::collections::{HashMap, HashSet};

use super::{ExportManifestTrack, PdbTrackRowData, load_table_columns, table_exists};
use crate::error::BackendResult;

/// eDB `content` columns the writers sync from the manifest.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const EDB_CONTENT_SYNCED_COLUMNS: &[&str] = &[
    "title",
    "titleForSearch",
    "subtitle",
    "bpmx100",
    "length",
    "trackNo",
    "discNo",
    "artist_id_artist",
    "artist_id_remixer",
    "artist_id_originalArtist",
    "artist_id_composer",
    "artist_id_lyricist",
    "album_id",
    "genre_id",
    "label_id",
    "key_id",
    "color_id",
    "image_id",
    "djComment",
    "rating",
    "releaseYear",
    "releaseDate",
    "dateCreated",
    "dateAdded",
    "path",
    "fileName",
    "fileSize",
    "fileType",
    "bitrate",
    "bitDepth",
    "samplingRate",
    "isrc",
    "djPlayCount",
    "kuvoDeliveryComment",
    "masterDbId",
    "masterContentId",
    "analysisDataFilePath",
    "contentLink",
];

/// eDB `content` columns not synced from a manifest field: the row key, and
/// the export's own bookkeeping, which the eDB writer sets only for tracks
/// exported from the library (`ExportManifestTrack::from_library`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const EDB_CONTENT_NOT_SYNCED_COLUMNS: &[&str] = &[
    "content_id",
    "isHotCueAutoLoadOn",
    "isKuvoDeliverStatusOn",
    "analysedBits",
    "hasModified",
    "cueUpdateCount",
    "analysisDataUpdateCount",
    "informationUpdateCount",
];

/// Every [`ExportManifestTrack`] field by role. Compile-time guard only.
#[allow(dead_code)]
fn manifest_track_fields_are_classified(track: &ExportManifestTrack) {
    let ExportManifestTrack {
        // Synced into both DBs (eDB column / PDB field).
        master_db_id: _,       // masterDbId / master_db_id
        master_content_id: _,  // masterContentId / master_content_id
        content_link: _,       // contentLink / content_link
        track_number: _,       // trackNo / track_number
        title: _,              // title / title
        artist: _,             // artist_id_artist / artist_id (by name)
        album: _,              // album_id / album_id (by name)
        bpm: _,                // bpmx100 / bpm
        key: _,                // key_id / key_id (by name)
        exported_path: _,      // path, fileName / file_path, file_name
        file_size_bytes: _,    // fileSize / file_size_bytes
        sample_rate_hz: _,     // samplingRate / sample_rate_hz
        bit_depth: _,          // bitDepth / bit_depth
        bitrate_kbps: _,       // bitrate / bitrate_kbps
        comment: _,            // djComment / dj_comment
        rating: _,             // rating / rating
        color_id: _,           // color_id / color_id
        genre: _,              // genre_id / genre_id (by name)
        genre_id: _,           // genre_id when no name
        isrc: _,               // isrc / isrc
        release_year: _,       // releaseYear / release_year
        release_date: _,       // releaseDate / release_date
        file_type: _,          // fileType / file_type
        artwork_path: _,       // image_id / artwork_id (by path)
        waveform_path: _,      // analysisDataFilePath / anlz_path
        duration_ms: _,        // length / duration_seconds
        date_added: _,         // dateAdded / date_added
        label: _,              // label_id / label_id (by name)
        label_id: _,           // label_id when no name
        remixer: _,            // artist_id_remixer / remixer_id (by name)
        artist_id_remixer: _,  // artist_id_remixer when no name
        composer: _,           // artist_id_composer / composer_id (by name)
        artist_id_composer: _, // artist_id_composer when no name
        // eDB only: the PDB track row has no slot for these.
        disc_number: _,               // discNo
        subtitle: _,                  // subtitle
        title_for_search: _,          // titleForSearch
        kuvo_delivery_comment: _,     // kuvoDeliveryComment
        dj_play_count: _,             // djPlayCount
        artist_id_lyricist: _,        // artist_id_lyricist
        artist_id_original_artist: _, // artist_id_originalArtist
        date_created: _,              // dateCreated
        cues: _,                      // cue table (from_library only)
        // Not synced: identity, file copying and writer control.
        id: _,
        position: _,
        source_path: _,
        file_timestamp_at: _,
        recorded_date: _,
        first_beat_ms: _,
        owns_exported_media: _,
        owns_artwork: _,
        owns_waveform: _,
        from_library: _,
        pdb_track_id: _,
        edb_content_id: _,
    } = track;
}

/// Every [`PdbTrackRowData`] field by role. Compile-time guard only.
#[allow(dead_code)]
fn pdb_track_row_fields_are_classified(row: &PdbTrackRowData) {
    let PdbTrackRowData {
        // Synced from the manifest (see above).
        content_link: _,
        sample_rate_hz: _,
        file_size_bytes: _,
        master_content_id: _,
        master_db_id: _,
        artist_id: _,
        album_id: _,
        artwork_id: _,
        key_id: _,
        genre_id: _,
        composer_id: _,
        label_id: _,
        remixer_id: _,
        bitrate_kbps: _,
        track_number: _,
        bpm: _,
        release_year: _,
        bit_depth: _,
        duration_seconds: _,
        file_type: _,
        isrc: _,
        date_added: _,
        release_date: _,
        dj_comment: _,
        file_name: _,
        color_id: _,
        rating: _,
        title: _,
        anlz_path: _,
        file_path: _,
        // Not synced: the row id, and row bytes kept from the existing row.
        id: _,
        header_flags_u32: _,
        publish_track_info_on: _,
        autoload_hotcues_on: _,
    } = row;
}

/// A `content` row's values the repair carries over, with the names its
/// dictionary ids point at. Missing or empty values are `None`.
#[derive(Debug, Default, Clone)]
pub(crate) struct EdbContentFields {
    pub(crate) content_id: i64,
    pub(crate) path: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) genre: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) remixer: Option<String>,
    pub(crate) composer: Option<String>,
    pub(crate) artwork_path: Option<String>,
    pub(crate) analysis_path: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) isrc: Option<String>,
    pub(crate) date_added: Option<String>,
    pub(crate) date_created: Option<String>,
    pub(crate) release_date: Option<String>,
    pub(crate) bpmx100: Option<i64>,
    pub(crate) length: Option<i64>,
    pub(crate) track_no: Option<i64>,
    pub(crate) disc_no: Option<i64>,
    pub(crate) rating: Option<i64>,
    pub(crate) color_id: Option<i64>,
    pub(crate) release_year: Option<i64>,
    pub(crate) bitrate: Option<i64>,
    pub(crate) sampling_rate: Option<i64>,
    pub(crate) bit_depth: Option<i64>,
    pub(crate) file_size: Option<i64>,
    pub(crate) file_type: Option<i64>,
    pub(crate) master_db_id: Option<i64>,
    pub(crate) master_content_id: Option<i64>,
    pub(crate) content_link: Option<i64>,
}

/// Every eDB `content` row's [`EdbContentFields`], by `content_id`. Reads
/// raw values: nothing is substituted for a missing one.
pub(crate) fn read_edb_content_fields(
    conn: &rusqlite::Connection,
) -> BackendResult<HashMap<i64, EdbContentFields>> {
    use rusqlite::types::Value;
    if !table_exists(conn, "content") {
        return Ok(HashMap::new());
    }
    let columns: HashSet<String> = load_table_columns(conn, "content")?.into_iter().collect();
    let col = |name: &str| {
        if columns.contains(name) {
            format!("c.\"{name}\"")
        } else {
            "NULL".to_string()
        }
    };
    let mut joins = Vec::<String>::new();
    let mut name_of = |table: &str, alias: &str, fk: &str| {
        if table_exists(conn, table) && columns.contains(fk) {
            joins.push(format!(
                "LEFT JOIN \"{table}\" {alias} ON {alias}.{table}_id = c.\"{fk}\""
            ));
            format!("{alias}.name")
        } else {
            "NULL".to_string()
        }
    };
    let artist = name_of("artist", "ar", "artist_id_artist");
    let remixer = if table_exists(conn, "artist") && columns.contains("artist_id_remixer") {
        joins.push("LEFT JOIN artist rx ON rx.artist_id = c.artist_id_remixer".to_string());
        "rx.name".to_string()
    } else {
        "NULL".to_string()
    };
    let composer = if table_exists(conn, "artist") && columns.contains("artist_id_composer") {
        joins.push("LEFT JOIN artist cp ON cp.artist_id = c.artist_id_composer".to_string());
        "cp.name".to_string()
    } else {
        "NULL".to_string()
    };
    let mut name_of = |table: &str, alias: &str, fk: &str| {
        if table_exists(conn, table) && columns.contains(fk) {
            joins.push(format!(
                "LEFT JOIN \"{table}\" {alias} ON {alias}.{table}_id = c.\"{fk}\""
            ));
            format!("{alias}.name")
        } else {
            "NULL".to_string()
        }
    };
    let album = name_of("album", "al", "album_id");
    let genre = name_of("genre", "ge", "genre_id");
    let label = name_of("label", "lb", "label_id");
    let key = name_of("key", "ky", "key_id");
    let artwork = if table_exists(conn, "image") && columns.contains("image_id") {
        joins.push("LEFT JOIN image im ON im.image_id = c.image_id".to_string());
        "im.path".to_string()
    } else {
        "NULL".to_string()
    };
    let sql = format!(
        "SELECT c.content_id, {}, {}, {artist}, {album}, {genre}, {label}, {key}, {remixer}, \
         {composer}, {artwork}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, \
         {}, {}, {}, {}, {} FROM content c {}",
        col("path"),
        col("title"),
        col("analysisDataFilePath"),
        col("djComment"),
        col("isrc"),
        col("dateAdded"),
        col("dateCreated"),
        col("releaseDate"),
        col("bpmx100"),
        col("length"),
        col("trackNo"),
        col("discNo"),
        col("rating"),
        col("color_id"),
        col("releaseYear"),
        col("bitrate"),
        col("samplingRate"),
        col("bitDepth"),
        col("fileSize"),
        col("fileType"),
        col("masterDbId"),
        col("masterContentId"),
        col("contentLink"),
        joins.join(" "),
    );
    let text = |v: Value| match v {
        Value::Text(t) => Some(t).filter(|t| !t.trim().is_empty()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(f) => Some(f.to_string()),
        _ => None,
    };
    let int = |v: Value| match v {
        Value::Integer(i) => Some(i),
        Value::Real(f) => Some(f as i64),
        Value::Text(t) => t.trim().parse::<i64>().ok(),
        _ => None,
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let v = |i: usize| row.get::<_, Value>(i);
        Ok(EdbContentFields {
            content_id: row.get(0)?,
            path: text(v(1)?),
            title: text(v(2)?),
            artist: text(v(3)?),
            album: text(v(4)?),
            genre: text(v(5)?),
            label: text(v(6)?),
            key: text(v(7)?),
            remixer: text(v(8)?),
            composer: text(v(9)?),
            artwork_path: text(v(10)?),
            analysis_path: text(v(11)?),
            comment: text(v(12)?),
            isrc: text(v(13)?),
            date_added: text(v(14)?),
            date_created: text(v(15)?),
            release_date: text(v(16)?),
            bpmx100: int(v(17)?),
            length: int(v(18)?),
            track_no: int(v(19)?),
            disc_no: int(v(20)?),
            rating: int(v(21)?),
            color_id: int(v(22)?),
            release_year: int(v(23)?),
            bitrate: int(v(24)?),
            sampling_rate: int(v(25)?),
            bit_depth: int(v(26)?),
            file_size: int(v(27)?),
            file_type: int(v(28)?),
            master_db_id: int(v(29)?),
            master_content_id: int(v(30)?),
            content_link: int(v(31)?),
        })
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let row = row?;
        out.insert(row.content_id, row);
    }
    Ok(out)
}

/// The text, or `None` when missing or blank.
fn present(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// The number, or `None` when missing or 0 (unknown on both sides).
fn known(v: Option<i64>) -> Option<i64> {
    v.filter(|v| *v != 0)
}

/// The synced fields that parity diagnostics report only as notes (beyond
/// the paths, dictionary ids, artwork, rating/colour and core metadata they
/// check), that differ between a track's eDB row and its PDB row. A value
/// missing on one side counts: the strict parity repair fills it.
pub(crate) fn note_field_differences(
    edb: &EdbContentFields,
    pdb: &crate::pdb_reader::PdbTrackRow,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> Vec<&'static str> {
    fn name(map: &HashMap<u32, String>, id: u32) -> Option<&str> {
        present(
            (id != 0)
                .then(|| map.get(&id).map(String::as_str))
                .flatten(),
        )
    }
    let text_differs = |e: Option<&str>, p: Option<&str>| present(e) != present(p);
    let num_differs = |e: Option<i64>, p: Option<i64>| known(e) != known(p);
    let mut out = Vec::new();
    let checks: [(&'static str, bool); 13] = [
        (
            "comment",
            text_differs(edb.comment.as_deref(), pdb.dj_comment.as_deref()),
        ),
        (
            "isrc",
            text_differs(edb.isrc.as_deref(), pdb.isrc.as_deref()),
        ),
        (
            "genre",
            text_differs(edb.genre.as_deref(), name(&parsed.genres, pdb.genre_id)),
        ),
        (
            "label",
            text_differs(edb.label.as_deref(), name(&parsed.labels, pdb.label_id)),
        ),
        (
            "remixer",
            text_differs(
                edb.remixer.as_deref(),
                name(&parsed.artists, pdb.remixer_id),
            ),
        ),
        (
            "composer",
            text_differs(
                edb.composer.as_deref(),
                name(&parsed.artists, pdb.composer_id),
            ),
        ),
        (
            "releaseYear",
            num_differs(edb.release_year, pdb.release_year.map(i64::from)),
        ),
        (
            "releaseDate",
            text_differs(edb.release_date.as_deref(), pdb.release_date.as_deref()),
        ),
        (
            "dateAdded",
            text_differs(edb.date_added.as_deref(), pdb.date_added.as_deref()),
        ),
        (
            "bitrate",
            num_differs(edb.bitrate, pdb.bitrate_kbps.map(i64::from)),
        ),
        (
            "sampleRate",
            num_differs(edb.sampling_rate, pdb.sample_rate_hz.map(i64::from)),
        ),
        (
            "bitDepth",
            num_differs(edb.bit_depth, pdb.bit_depth.map(i64::from)),
        ),
        (
            "fileSize",
            num_differs(edb.file_size, pdb.file_size_bytes.map(i64::from)),
        ),
    ];
    for (field, differs) in checks {
        if differs {
            out.push(field);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edb_template_content_column_is_mapped_or_listed_as_not_synced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = rusqlite::Connection::open(dir.path().join("t.db")).expect("open");
        conn.execute_batch(crate::service::usb_utils::EDB_SCHEMA)
            .expect("template schema");
        let columns = load_table_columns(&conn, "content").expect("columns");
        let synced: HashSet<&str> = EDB_CONTENT_SYNCED_COLUMNS.iter().copied().collect();
        let not_synced: HashSet<&str> = EDB_CONTENT_NOT_SYNCED_COLUMNS.iter().copied().collect();
        assert!(synced.is_disjoint(&not_synced), "a column is in both lists");
        let unclassified: Vec<&String> = columns
            .iter()
            .filter(|c| !synced.contains(c.as_str()) && !not_synced.contains(c.as_str()))
            .collect();
        assert!(
            unclassified.is_empty(),
            "eDB content columns neither synced nor listed as not synced: {unclassified:?}"
        );
        let stale: Vec<&&str> = synced
            .iter()
            .chain(not_synced.iter())
            .filter(|c| !columns.iter().any(|col| col == **c))
            .collect();
        assert!(
            stale.is_empty(),
            "listed columns missing from the template: {stale:?}"
        );
    }
}
