use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::path::{Path, PathBuf};

use backend::pdb_reader::{parse_pdb, parse_pdb_track_debug_rows};
use rusqlite::{Connection, types::ValueRef};
use serde::Serialize;

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

#[derive(Debug, Serialize)]
struct DumpOutput {
    usb_root: String,
    edb_tables: Vec<DumpTable>,
    pdb_tables: Vec<DumpTable>,
    pdb_field_mapping: Vec<PdbFieldMappingStatus>,
    track_comparisons: Vec<TrackComparison>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DumpTable {
    name: String,
    columns: Vec<String>,
    rows: Vec<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct TrackComparison {
    track_key: String,
    playlist_names: Vec<String>,
    edb_fields: BTreeMap<String, String>,
    pdb_fields: BTreeMap<String, String>,
    reference_documented_debug: ReferenceDocumentedDebug,
    mismatched_fields: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReferenceDocumentedDebug {
    edb_reference_fields: BTreeMap<String, String>,
    pdb_candidate_fixed_fields: BTreeMap<String, String>,
    pdb_candidate_string_fields: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
struct PdbFieldMappingStatus {
    field: String,
    kind: String,
    mapping_status: String,
    edb_column: String,
    notes: String,
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: cargo run --bin dump_usb_db_compare -- <usb_root>");
        std::process::exit(2);
    }

    let usb_root = PathBuf::from(&args[1]);
    let vendor_db = usb_root.join("PIONEER").join("rekordbox");
    let export_db = vendor_db.join("exportLibrary.db");
    let pdb_file = vendor_db.join("export.pdb");

    if !export_db.is_file() {
        eprintln!("error: eDB not found at {}", export_db.display());
        std::process::exit(1);
    }
    if !pdb_file.is_file() {
        eprintln!("error: PDB not found at {}", pdb_file.display());
        std::process::exit(1);
    }

    let mut warnings = Vec::<String>::new();
    let conn = match open_export_db(&export_db, &mut warnings) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("error: failed to open {}: {err}", export_db.display());
            std::process::exit(1);
        }
    };
    let parsed = match parse_pdb(&pdb_file) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("error: failed to parse {}: {err:?}", pdb_file.display());
            std::process::exit(1);
        }
    };
    let track_debug_rows = match parse_pdb_track_debug_rows(&pdb_file) {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!(
                "error: failed to parse track debug rows from {}: {err:?}",
                pdb_file.display()
            );
            std::process::exit(1);
        }
    };
    warnings.extend(parsed.warnings.clone());

    let edb_tables = dump_sqlite_tables(&conn);
    let pdb_tables = dump_pdb_tables(&parsed, &track_debug_rows);
    let pdb_field_mapping = build_pdb_field_mapping_status();
    let track_comparisons = build_track_comparisons(&conn, &parsed, &track_debug_rows);

    let output = DumpOutput {
        usb_root: usb_root.to_string_lossy().to_string(),
        edb_tables,
        pdb_tables,
        pdb_field_mapping,
        track_comparisons,
        warnings,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("serialize dump output")
    );
}

fn open_export_db(path: &Path, warnings: &mut Vec<String>) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    let has_schema = conn
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if has_schema > 0 {
        warnings.push("eDB opened without SQLCipher key".to_string());
        return Ok(conn);
    }

    let keys = [DEFAULT_USB_EXPORT_KEY, DEFAULT_MASTER_KEY];
    for key in keys {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        if conn.execute_batch(&format!("PRAGMA key='{key}';")).is_err() {
            continue;
        }
        let unlocked = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if unlocked > 0 {
            warnings.push("eDB opened with SQLCipher key".to_string());
            return Ok(conn);
        }
    }

    Err("not readable as plain sqlite or with known SQLCipher keys".to_string())
}

fn dump_sqlite_tables(conn: &Connection) -> Vec<DumpTable> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare sqlite_master");
    let table_names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query sqlite_master")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect table names");

    table_names
        .into_iter()
        .map(|name| dump_sqlite_table(conn, &name))
        .collect()
}

fn dump_sqlite_table(conn: &Connection, table: &str) -> DumpTable {
    let ident = format!("\"{}\"", table.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({ident})"))
        .expect("prepare table_info");
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns");

    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {ident}"))
        .expect("prepare select all");
    let rows = stmt
        .query_map([], |row| {
            let mut map = BTreeMap::<String, String>::new();
            for (idx, column) in columns.iter().enumerate() {
                map.insert(column.clone(), render_value_ref(row.get_ref(idx)?));
            }
            Ok(map)
        })
        .expect("query table rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect table rows");

    DumpTable {
        name: table.to_string(),
        columns,
        rows,
    }
}

fn dump_pdb_tables(
    parsed: &backend::pdb_reader::ParsedPdb,
    track_debug_rows: &[backend::pdb_reader::PdbTrackDebugRow],
) -> Vec<DumpTable> {
    let mut out = Vec::<DumpTable>::new();

    out.push(DumpTable {
        name: "tracks".to_string(),
        columns: vec![
            "id".to_string(),
            "artist_id".to_string(),
            "album_id".to_string(),
            "artwork_id".to_string(),
            "key_id".to_string(),
            "track_number".to_string(),
            "tempo_x100".to_string(),
            "duration_seconds".to_string(),
            "title".to_string(),
            "anlz_path".to_string(),
            "track_file_path".to_string(),
        ],
        rows: parsed
            .tracks
            .iter()
            .map(|track| {
                let debug = track_debug_rows.iter().find(|row| row.id == track.id);
                let mut row = BTreeMap::from([
                    ("id".to_string(), track.id.to_string()),
                    ("artist_id".to_string(), track.artist_id.to_string()),
                    ("album_id".to_string(), track.album_id.to_string()),
                    ("artwork_id".to_string(), track.artwork_id.to_string()),
                    ("key_id".to_string(), track.key_id.to_string()),
                    ("track_number".to_string(), track.track_number.to_string()),
                    ("tempo_x100".to_string(), track.tempo_x100.to_string()),
                    (
                        "duration_seconds".to_string(),
                        track
                            .duration_seconds
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    ),
                    ("title".to_string(), track.title.clone()),
                    ("anlz_path".to_string(), track.anlz_path.clone()),
                    ("track_file_path".to_string(), track.track_file_path.clone()),
                ]);
                if let Some(debug) = debug {
                    row.insert("row_len".to_string(), debug.row_len.to_string());
                    row.insert("raw_hex".to_string(), debug.raw_hex.clone());
                    row.insert("fixed_block_hex".to_string(), debug.fixed_block_hex.clone());
                    for (key, value) in &debug.fixed_fields {
                        row.insert(format!("fixed::{key}"), value.clone());
                    }
                    for slot in &debug.string_slots {
                        row.insert(
                            format!("string_offset[{}]::{}", slot.index, slot.label),
                            slot.offset.to_string(),
                        );
                        row.insert(
                            format!("string_raw[{}]::{}", slot.index, slot.label),
                            slot.raw_hex.clone(),
                        );
                        row.insert(
                            format!("string_value[{}]::{}", slot.index, slot.label),
                            slot.decoded_value.clone().unwrap_or_default(),
                        );
                    }
                }
                row
            })
            .collect(),
    });

    out.push(map_u32_string_table("artists", &parsed.artists));
    out.push(map_u32_string_table("albums", &parsed.albums));
    out.push(map_u32_string_table("keys", &parsed.keys));
    out.push(map_u32_string_table("artworks", &parsed.artworks));

    out.push(DumpTable {
        name: "playlist_tree".to_string(),
        columns: vec![
            "id".to_string(),
            "parent_id".to_string(),
            "sort_order".to_string(),
            "row_is_folder".to_string(),
            "name".to_string(),
        ],
        rows: parsed
            .playlist_tree
            .iter()
            .map(|row| {
                BTreeMap::from([
                    ("id".to_string(), row.id.to_string()),
                    ("parent_id".to_string(), row.parent_id.to_string()),
                    ("sort_order".to_string(), row.sort_order.to_string()),
                    ("row_is_folder".to_string(), row.row_is_folder.to_string()),
                    ("name".to_string(), row.name.clone()),
                ])
            })
            .collect(),
    });

    out.push(DumpTable {
        name: "playlist_entries".to_string(),
        columns: vec![
            "entry_index".to_string(),
            "track_id".to_string(),
            "playlist_id".to_string(),
        ],
        rows: parsed
            .playlist_entries
            .iter()
            .map(|row| {
                BTreeMap::from([
                    ("entry_index".to_string(), row.entry_index.to_string()),
                    ("track_id".to_string(), row.track_id.to_string()),
                    ("playlist_id".to_string(), row.playlist_id.to_string()),
                ])
            })
            .collect(),
    });

    out.push(DumpTable {
        name: "history_playlists".to_string(),
        columns: vec!["id".to_string(), "name".to_string()],
        rows: parsed
            .history_playlists
            .iter()
            .map(|row| {
                BTreeMap::from([
                    ("id".to_string(), row.id.to_string()),
                    ("name".to_string(), row.name.clone()),
                ])
            })
            .collect(),
    });

    out.push(DumpTable {
        name: "history_entries".to_string(),
        columns: vec![
            "track_id".to_string(),
            "playlist_id".to_string(),
            "entry_index".to_string(),
        ],
        rows: parsed
            .history_entries
            .iter()
            .map(|row| {
                BTreeMap::from([
                    (
                        "track_id".to_string(),
                        row.track_id.map(|v| v.to_string()).unwrap_or_default(),
                    ),
                    ("playlist_id".to_string(), row.playlist_id.to_string()),
                    ("entry_index".to_string(), row.entry_index.to_string()),
                ])
            })
            .collect(),
    });

    out.push(DumpTable {
        name: "history_rows".to_string(),
        columns: vec!["date".to_string(), "num".to_string()],
        rows: parsed
            .history_rows
            .iter()
            .map(|row| {
                BTreeMap::from([
                    ("date".to_string(), row.date.clone().unwrap_or_default()),
                    ("num".to_string(), row.num.clone().unwrap_or_default()),
                ])
            })
            .collect(),
    });

    out
}

fn map_u32_string_table(name: &str, rows: &HashMap<u32, String>) -> DumpTable {
    let mut items = rows.iter().collect::<Vec<_>>();
    items.sort_by_key(|(id, _)| **id);
    DumpTable {
        name: name.to_string(),
        columns: vec!["id".to_string(), "value".to_string()],
        rows: items
            .into_iter()
            .map(|(id, value)| {
                BTreeMap::from([
                    ("id".to_string(), id.to_string()),
                    ("value".to_string(), value.clone()),
                ])
            })
            .collect(),
    }
}

fn build_track_comparisons(
    conn: &Connection,
    parsed: &backend::pdb_reader::ParsedPdb,
    track_debug_rows: &[backend::pdb_reader::PdbTrackDebugRow],
) -> Vec<TrackComparison> {
    let track_debug_by_id = track_debug_rows
        .iter()
        .map(|row| (row.id, row))
        .collect::<HashMap<_, _>>();
    let edb_rows = load_playlist_linked_edb_rows(conn);
    let pdb_by_key = parsed
        .tracks
        .iter()
        .map(|track| {
            let debug = track_debug_by_id.get(&track.id).copied();
            let key = normalize_key(&track.track_file_path);
            let mut fields = BTreeMap::from([
                ("title".to_string(), track.title.clone()),
                (
                    "artist_id".to_string(),
                    if track.artist_id == 0 {
                        String::new()
                    } else {
                        track.artist_id.to_string()
                    },
                ),
                (
                    "artist_name".to_string(),
                    parsed
                        .artists
                        .get(&track.artist_id)
                        .cloned()
                        .unwrap_or_default(),
                ),
                (
                    "album_id".to_string(),
                    if track.album_id == 0 {
                        String::new()
                    } else {
                        track.album_id.to_string()
                    },
                ),
                (
                    "album_name".to_string(),
                    parsed
                        .albums
                        .get(&track.album_id)
                        .cloned()
                        .unwrap_or_default(),
                ),
                (
                    "key_id".to_string(),
                    if track.key_id == 0 {
                        String::new()
                    } else {
                        track.key_id.to_string()
                    },
                ),
                (
                    "key_name".to_string(),
                    parsed.keys.get(&track.key_id).cloned().unwrap_or_default(),
                ),
                (
                    "artwork_id".to_string(),
                    if track.artwork_id == 0 {
                        String::new()
                    } else {
                        track.artwork_id.to_string()
                    },
                ),
                (
                    "artwork_path".to_string(),
                    parsed
                        .artworks
                        .get(&track.artwork_id)
                        .cloned()
                        .unwrap_or_default(),
                ),
                ("track_number".to_string(), track.track_number.to_string()),
                ("tempo_x100".to_string(), track.tempo_x100.to_string()),
                (
                    "duration_seconds".to_string(),
                    track
                        .duration_seconds
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                ),
                ("analysis_path".to_string(), track.anlz_path.clone()),
                ("media_path".to_string(), track.track_file_path.clone()),
            ]);
            if let Some(debug) = debug {
                for slot in &debug.string_slots {
                    fields.insert(
                        format!("pdb_string_offset::{}", slot.label),
                        slot.offset.to_string(),
                    );
                    fields.insert(
                        format!("pdb_string_raw::{}", slot.label),
                        slot.raw_hex.clone(),
                    );
                    fields.insert(
                        format!("pdb_string_value::{}", slot.label),
                        slot.decoded_value.clone().unwrap_or_default(),
                    );
                }
                for (field, value) in &debug.fixed_fields {
                    fields.insert(format!("pdb_fixed::{field}"), value.clone());
                }
            }
            (key, fields)
        })
        .collect::<HashMap<_, _>>();

    let mut out = Vec::<TrackComparison>::new();
    for (key, playlist_names, edb_fields) in edb_rows {
        let pdb_fields = pdb_by_key.get(&key).cloned().unwrap_or_default();
        let reference_documented_debug = build_reference_documented_debug(&edb_fields, &pdb_fields);
        let mut mismatched_fields = Vec::<String>::new();
        for (edb_field, pdb_field) in [
            ("title", "title"),
            ("artist_id_artist", "artist_id"),
            ("artist_name", "artist_name"),
            ("album_id", "album_id"),
            ("album_name", "album_name"),
            ("key_id", "key_id"),
            ("key_name", "key_name"),
            ("image_id", "artwork_id"),
            ("image_path", "artwork_path"),
            ("trackNo", "track_number"),
            ("bpmx100", "tempo_x100"),
            ("length", "duration_seconds"),
            ("analysisDataFilePath", "analysis_path"),
            ("path", "media_path"),
            ("isrc", "pdb_string_value::isrc"),
            ("fileName", "pdb_string_value::filename"),
            ("dateAdded", "pdb_string_value::date_added"),
            ("releaseDate", "pdb_string_value::release_date"),
            ("contentLink", "pdb_fixed::content_link_u32"),
            ("masterContentId", "pdb_fixed::master_content_id_u32"),
            ("masterDbId", "pdb_fixed::master_db_id_u32"),
            ("samplingRate", "pdb_fixed::sample_rate_hz"),
            ("fileSize", "pdb_fixed::file_size_bytes"),
            ("bitrate", "pdb_fixed::bitrate_kbps"),
            ("releaseYear", "pdb_fixed::release_year_u16"),
            ("bitDepth", "pdb_fixed::bit_depth_u16"),
            ("fileType", "pdb_fixed::file_type_u16"),
        ] {
            let left = edb_fields.get(edb_field).cloned().unwrap_or_default();
            let right = pdb_fields.get(pdb_field).cloned().unwrap_or_default();
            if left != right {
                mismatched_fields.push(format!("{edb_field} != {pdb_field}"));
            }
        }
        for (edb_field, pdb_field) in [
            (
                "isKuvoDeliverStatusOn",
                "pdb_string_value::publish_track_info",
            ),
            ("isHotCueAutoLoadOn", "pdb_string_value::autoload_hotcues"),
        ] {
            let left = edb_fields.get(edb_field).cloned().unwrap_or_default();
            let right = pdb_fields.get(pdb_field).cloned().unwrap_or_default();
            if !matches_toggle_value(&left, &right) {
                mismatched_fields.push(format!("{edb_field} != {pdb_field}"));
            }
        }
        let pdb_comment = pdb_fields
            .get("pdb_string_value::comment")
            .cloned()
            .unwrap_or_default();
        if !pdb_comment.is_empty() {
            let edb_dj_comment = edb_fields.get("djComment").cloned().unwrap_or_default();
            let edb_comment = edb_fields.get("comment").cloned().unwrap_or_default();
            let comment_matches = pdb_comment == edb_dj_comment
                || (!edb_comment.is_empty() && pdb_comment == edb_comment);
            if !comment_matches {
                mismatched_fields
                    .push("djComment/comment != pdb_string_value::comment".to_string());
            }
        }
        out.push(TrackComparison {
            track_key: key,
            playlist_names,
            edb_fields,
            pdb_fields,
            reference_documented_debug,
            mismatched_fields,
        });
    }
    out.sort_by(|a, b| a.track_key.cmp(&b.track_key));
    out
}

fn build_reference_documented_debug(
    edb_fields: &BTreeMap<String, String>,
    pdb_fields: &BTreeMap<String, String>,
) -> ReferenceDocumentedDebug {
    let mut edb_reference_fields = BTreeMap::<String, String>::new();
    for field in [
        "discNo",
        "djPlayCount",
        "rating",
        "color_id",
        "genre_id",
        "label_id",
        "artist_id_lyricist",
        "artist_id_originalArtist",
        "artist_id_remixer",
        "artist_id_composer",
    ] {
        edb_reference_fields.insert(
            field.to_string(),
            edb_fields.get(field).cloned().unwrap_or_default(),
        );
    }

    let mut pdb_candidate_fixed_fields = BTreeMap::<String, String>::new();
    for field in [
        "pdb_fixed::unknown_fixed_00_27_hex",
        "pdb_fixed::unknown_fixed_36_51_hex",
        "pdb_fixed::unknown_fixed_60_63_hex",
        "pdb_fixed::unknown_fixed_76_83_hex",
        "pdb_fixed::unknown_fixed_86_93_hex",
    ] {
        pdb_candidate_fixed_fields.insert(
            field.trim_start_matches("pdb_fixed::").to_string(),
            pdb_fields.get(field).cloned().unwrap_or_default(),
        );
    }

    let mut pdb_candidate_string_fields = BTreeMap::<String, String>::new();
    for label in ["lyricist", "message", "mix_name", "unknown_string_7"] {
        pdb_candidate_string_fields.insert(
            format!("{label}::decoded"),
            pdb_fields
                .get(&format!("pdb_string_value::{label}"))
                .cloned()
                .unwrap_or_default(),
        );
        pdb_candidate_string_fields.insert(
            format!("{label}::raw"),
            pdb_fields
                .get(&format!("pdb_string_raw::{label}"))
                .cloned()
                .unwrap_or_default(),
        );
    }

    ReferenceDocumentedDebug {
        edb_reference_fields,
        pdb_candidate_fixed_fields,
        pdb_candidate_string_fields,
    }
}

fn matches_toggle_value(edb_value: &str, pdb_value: &str) -> bool {
    let left = edb_value.trim().to_ascii_lowercase();
    let right = pdb_value.trim().to_ascii_lowercase();
    if left.is_empty() && right.is_empty() {
        return true;
    }
    let left_on = matches!(left.as_str(), "1" | "true" | "on" | "yes");
    let right_on = matches!(right.as_str(), "1" | "true" | "on" | "yes");
    let left_off = matches!(left.as_str(), "0" | "false" | "off" | "no");
    let right_off = matches!(right.as_str(), "0" | "false" | "off" | "no");
    (left_on && right_on) || (left_off && right_off) || left == right
}

fn load_playlist_linked_edb_rows(
    conn: &Connection,
) -> Vec<(String, Vec<String>, BTreeMap<String, String>)> {
    let columns = table_columns(conn, "content");
    let select_columns = columns
        .iter()
        .map(|column| format!("c.\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r#"
        SELECT p.name,
               {select_columns},
               artist.name AS artist_name,
               album.name AS album_name,
               img.path AS image_path,
               k.name AS key_name
        FROM playlist_content pc
        JOIN playlist p ON p.playlist_id = pc.playlist_id
        JOIN content c ON c.content_id = pc.content_id
        LEFT JOIN artist artist ON artist.artist_id = c.artist_id_artist
        LEFT JOIN album album ON album.album_id = c.album_id
        LEFT JOIN image img ON img.image_id = c.image_id
        LEFT JOIN "key" k ON k.key_id = c.key_id
        ORDER BY p.name ASC, pc.sequenceNo ASC, c.content_id ASC
        "#
    );

    let mut stmt = conn
        .prepare(&sql)
        .expect("prepare playlist-linked content query");
    let mut rows = stmt.query([]).expect("query playlist-linked content");
    let mut by_key = BTreeMap::<String, (BTreeSet<String>, BTreeMap<String, String>)>::new();
    while let Some(row) = rows.next().expect("next playlist-linked row") {
        let playlist_name: String = row.get(0).expect("playlist name");
        let mut fields = BTreeMap::<String, String>::new();
        for (idx, column) in columns.iter().enumerate() {
            fields.insert(
                column.clone(),
                render_value_ref(row.get_ref(idx + 1).expect("content value")),
            );
        }
        let base = 1 + columns.len();
        fields.insert(
            "artist_name".to_string(),
            render_value_ref(row.get_ref(base).expect("artist name")),
        );
        fields.insert(
            "album_name".to_string(),
            render_value_ref(row.get_ref(base + 1).expect("album name")),
        );
        fields.insert(
            "image_path".to_string(),
            render_value_ref(row.get_ref(base + 2).expect("image path")),
        );
        fields.insert(
            "key_name".to_string(),
            render_value_ref(row.get_ref(base + 3).expect("key name")),
        );
        let key = normalize_key(fields.get("path").map(String::as_str).unwrap_or_default());
        let entry = by_key
            .entry(key)
            .or_insert_with(|| (BTreeSet::new(), fields.clone()));
        entry.0.insert(playlist_name);
        entry.1 = fields;
    }

    by_key
        .into_iter()
        .map(|(key, (playlist_names, fields))| (key, playlist_names.into_iter().collect(), fields))
        .collect()
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let ident = format!("\"{}\"", table.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({ident})"))
        .expect("prepare table_info");
    stmt.query_map([], |row| row.get::<_, String>(1))
        .expect("query table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect table columns")
}

fn normalize_key(value: &str) -> String {
    value.trim().replace('\\', "/").to_lowercase()
}

fn render_value_ref(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => String::new(),
        ValueRef::Integer(v) => v.to_string(),
        ValueRef::Real(v) => v.to_string(),
        ValueRef::Text(v) => String::from_utf8_lossy(v).into_owned(),
        ValueRef::Blob(v) => format!("<blob:{} bytes>", v.len()),
    }
}

fn build_pdb_field_mapping_status() -> Vec<PdbFieldMappingStatus> {
    let mut out = vec![
        // Mapped fixed fields
        mapped(
            "header_flags_u32",
            "fixed",
            "",
            "Structural row header flags.",
        ),
        mapped(
            "content_link_u32",
            "fixed",
            "contentLink",
            "Core identity/export linkage.",
        ),
        mapped(
            "sample_rate_hz",
            "fixed",
            "samplingRate",
            "Audio technical field.",
        ),
        mapped(
            "file_size_bytes",
            "fixed",
            "fileSize",
            "Audio technical field.",
        ),
        mapped(
            "master_content_id_u32",
            "fixed",
            "masterContentId",
            "Export identity field.",
        ),
        mapped(
            "master_db_id_u32",
            "fixed",
            "masterDbId",
            "Export identity field.",
        ),
        mapped(
            "artwork_id",
            "fixed",
            "image_id",
            "Dictionary-linked artwork id.",
        ),
        mapped("key_id", "fixed", "key_id", "Dictionary-linked key id."),
        mapped("bitrate_kbps", "fixed", "bitrate", "Audio technical field."),
        mapped("track_number", "fixed", "trackNo", "Core parity field."),
        mapped("tempo_x100", "fixed", "bpmx100", "Core parity field."),
        mapped(
            "album_id",
            "fixed",
            "album_id",
            "Dictionary-linked album id.",
        ),
        mapped(
            "artist_id",
            "fixed",
            "artist_id_artist",
            "Dictionary-linked artist id.",
        ),
        mapped("track_id", "fixed", "content_id", "Core row identity."),
        mapped(
            "release_year_u16",
            "fixed",
            "releaseYear",
            "Optional parity field.",
        ),
        mapped(
            "bit_depth_u16",
            "fixed",
            "bitDepth",
            "Audio technical field.",
        ),
        mapped(
            "duration_seconds_u16",
            "fixed",
            "length",
            "Core parity field.",
        ),
        mapped(
            "file_type_u16",
            "fixed",
            "fileType",
            "Audio technical field.",
        ),
        // Informational-only fixed ranges
        informational(
            "unknown_fixed_00_27_hex",
            "fixed_raw",
            "",
            "Debug raw view of mixed named/unknown bytes.",
        ),
        informational(
            "unknown_fixed_36_47_hex",
            "fixed_raw",
            "",
            "Debug raw view for unresolved fixed range.",
        ),
        informational(
            "unknown_fixed_60_63_hex",
            "fixed_raw",
            "",
            "Debug raw view for unresolved fixed range.",
        ),
        informational(
            "unknown_fixed_76_79_hex",
            "fixed_raw",
            "",
            "Debug raw view for unresolved fixed range.",
        ),
        informational(
            "unknown_fixed_86_89_hex",
            "fixed_raw",
            "",
            "Debug raw view for unresolved fixed range.",
        ),
        informational(
            "unknown_fixed_92_93_hex",
            "fixed_raw",
            "",
            "Debug raw view for unresolved fixed range.",
        ),
        // Mapped string slots
        mapped("isrc", "string_slot", "isrc", "Dedicated ISRC slot."),
        mapped(
            "publish_track_info",
            "string_slot",
            "isKuvoDeliverStatusOn",
            "Mapped toggle slot.",
        ),
        mapped(
            "autoload_hotcues",
            "string_slot",
            "isHotCueAutoLoadOn",
            "Mapped toggle slot.",
        ),
        mapped(
            "date_added",
            "string_slot",
            "dateAdded",
            "Mapped date slot.",
        ),
        mapped(
            "release_date",
            "string_slot",
            "releaseDate",
            "Mapped date slot.",
        ),
        mapped(
            "analysis_path_start",
            "string_slot",
            "analysisDataFilePath",
            "Path range start.",
        ),
        mapped(
            "analysis_path_end",
            "string_slot",
            "analysisDataFilePath",
            "Path range end.",
        ),
        mapped(
            "comment",
            "string_slot",
            "djComment/comment",
            "Comment slot.",
        ),
        mapped("title_start", "string_slot", "title", "Title range start."),
        mapped("title_end", "string_slot", "title", "Title range end."),
        mapped("filename", "string_slot", "fileName", "Filename slot."),
        mapped(
            "track_file_path",
            "string_slot",
            "path",
            "Core media path slot.",
        ),
        // Informational-only unresolved string slots
        informational(
            "lyricist",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_2",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_3",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_4",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "message",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_5",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_6",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "mix_name",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        informational(
            "unknown_string_7",
            "string_slot",
            "",
            "Slot exists; meaning not pinned.",
        ),
        // Mapping-unknown reference-derived targets
        unknown(
            "composer_id",
            "reference_target",
            "artist_id_composer",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "original_artist_id",
            "reference_target",
            "artist_id_originalArtist",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "label_id",
            "reference_target",
            "label_id",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "remixer_id",
            "reference_target",
            "artist_id_remixer",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "genre_id",
            "reference_target",
            "genre_id",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "disc_number",
            "reference_target",
            "discNo",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "play_count",
            "reference_target",
            "djPlayCount",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "color_id",
            "reference_target",
            "color_id",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "rating",
            "reference_target",
            "rating",
            "Reference-documented target; exact PDB placement unresolved.",
        ),
        unknown(
            "analyze_date",
            "reference_target",
            "",
            "Reference-documented string target; exact slot unresolved.",
        ),
        unknown(
            "unknown_string_8",
            "reference_target",
            "",
            "Reference-documented string target; exact slot unresolved.",
        ),
    ];
    out.sort_by(|a, b| a.field.cmp(&b.field));
    out
}

fn mapped(field: &str, kind: &str, edb_column: &str, notes: &str) -> PdbFieldMappingStatus {
    PdbFieldMappingStatus {
        field: field.to_string(),
        kind: kind.to_string(),
        mapping_status: "mapped".to_string(),
        edb_column: edb_column.to_string(),
        notes: notes.to_string(),
    }
}

fn informational(field: &str, kind: &str, edb_column: &str, notes: &str) -> PdbFieldMappingStatus {
    PdbFieldMappingStatus {
        field: field.to_string(),
        kind: kind.to_string(),
        mapping_status: "informational-only".to_string(),
        edb_column: edb_column.to_string(),
        notes: notes.to_string(),
    }
}

fn unknown(field: &str, kind: &str, edb_column: &str, notes: &str) -> PdbFieldMappingStatus {
    PdbFieldMappingStatus {
        field: field.to_string(),
        kind: kind.to_string(),
        mapping_status: "mapping unknown".to_string(),
        edb_column: edb_column.to_string(),
        notes: notes.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::build_pdb_field_mapping_status;

    #[test]
    fn field_mapping_status_contains_all_required_status_classes() {
        let rows = build_pdb_field_mapping_status();
        assert!(rows.iter().any(|r| r.mapping_status == "mapped"));
        assert!(
            rows.iter()
                .any(|r| r.mapping_status == "informational-only")
        );
        assert!(rows.iter().any(|r| r.mapping_status == "mapping unknown"));
    }

    #[test]
    fn field_mapping_status_includes_core_known_fields() {
        let rows = build_pdb_field_mapping_status();
        let has_track_id = rows.iter().any(|r| {
            r.field == "track_id" && r.mapping_status == "mapped" && r.edb_column == "content_id"
        });
        let has_unknown_fixed = rows.iter().any(|r| {
            r.field == "unknown_fixed_00_27_hex" && r.mapping_status == "informational-only"
        });
        let has_reference_unknown = rows.iter().any(|r| {
            r.field == "composer_id"
                && r.mapping_status == "mapping unknown"
                && r.edb_column == "artist_id_composer"
        });
        assert!(has_track_id, "missing mapped track_id -> content_id");
        assert!(
            has_unknown_fixed,
            "missing informational-only unresolved fixed range"
        );
        assert!(
            has_reference_unknown,
            "missing mapping-unknown reference target"
        );
    }
}
