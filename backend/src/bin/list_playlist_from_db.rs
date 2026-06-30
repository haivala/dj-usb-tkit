use std::env;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::pdb_reader::parse_pdb;
use rusqlite::{Connection, types::ValueRef};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    eprintln!(
        "usage: cargo run --bin list_playlist_from_db -- <app|edb|pdb> <data_dir|backend.db|usb_root|exportLibrary.db|export.pdb>"
    );
    std::process::exit(2);
}

fn resolve_app_data_dir(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("backend.db"))
        .unwrap_or(false)
    {
        return input
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| fail("backend.db path must have a parent directory"));
    }
    input.to_path_buf()
}

fn resolve_edb_path(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("exportlibrary.db"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input
        .join("PIONEER")
        .join("rekordbox")
        .join("exportLibrary.db")
}

fn resolve_pdb_path(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("export.pdb"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input.join("PIONEER").join("rekordbox").join("export.pdb")
}

fn open_edb_with_known_keys(path: &Path) -> Result<Connection, String> {
    let open_plain = || Connection::open(path).map_err(|e| e.to_string());
    let has_schema = |conn: &Connection| {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
            > 0
    };

    let plain = open_plain()?;
    if has_schema(&plain) {
        return Ok(plain);
    }

    let mut keys = vec![
        DEFAULT_MASTER_KEY.to_string(),
        DEFAULT_USB_EXPORT_KEY.to_string(),
    ];
    if let Ok(extra) = env::var("USB_DB_KEY") {
        let trimmed = extra.trim();
        if !trimmed.is_empty() && !keys.iter().any(|k| k == trimmed) {
            keys.insert(0, trimmed.to_string());
        }
    }

    for key in keys {
        let conn = open_plain()?;
        if conn.execute_batch(&format!("PRAGMA key='{key}';")).is_err() {
            continue;
        }
        if has_schema(&conn) {
            return Ok(conn);
        }
    }

    Err("not readable as plain sqlite or with known keys".to_string())
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(Result::ok).collect()
}

fn value_to_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
        ValueRef::Blob(b) => format!("<blob:{}>", b.len()),
    }
}

fn list_app_playlists(input: &Path) {
    let data_dir = resolve_app_data_dir(input);
    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));
    let list = backend.list_playlists();
    if !list.ok {
        fail(format!(
            "list_playlists failed: {}",
            list.error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }
    let data = list
        .data
        .unwrap_or_else(|| fail("missing list_playlists data"));
    for p in data.items {
        println!("playlist|db=app|id={}|name={}", p.id, p.name);
    }
}

fn list_edb_playlists(input: &Path) {
    let edb_path = resolve_edb_path(input);
    if !edb_path.is_file() {
        fail(format!("eDB not found: {}", edb_path.display()));
    }
    let conn = open_edb_with_known_keys(&edb_path)
        .unwrap_or_else(|err| fail(format!("open eDB failed: {err}")));

    let cols = table_columns(&conn, "playlist");
    if cols.is_empty() {
        fail("playlist table missing or unreadable");
    }

    let id_col = if cols.iter().any(|c| c == "playlist_id") {
        "playlist_id"
    } else {
        fail("playlist table missing playlist_id column");
    };
    let name_col = if cols.iter().any(|c| c == "name") {
        "name"
    } else {
        fail("playlist table missing name column");
    };
    let parent_col = if cols.iter().any(|c| c == "parent_id") {
        Some("parent_id")
    } else {
        None
    };
    let sort_col = if cols.iter().any(|c| c == "sort_order") {
        Some("sort_order")
    } else if cols.iter().any(|c| c == "seq") {
        Some("seq")
    } else {
        None
    };
    let attr_col = if cols.iter().any(|c| c == "attribute") {
        Some("attribute")
    } else {
        None
    };

    let select_parts = vec![
        format!("{id_col} AS id"),
        format!("{name_col} AS name"),
        parent_col
            .map(|c| format!("{c} AS parent_id"))
            .unwrap_or_else(|| "NULL AS parent_id".to_string()),
        sort_col
            .map(|c| format!("{c} AS sort_order"))
            .unwrap_or_else(|| "NULL AS sort_order".to_string()),
        attr_col
            .map(|c| format!("{c} AS attribute"))
            .unwrap_or_else(|| "NULL AS attribute".to_string()),
    ];
    let sql = format!(
        "SELECT {} FROM playlist ORDER BY name COLLATE NOCASE, id",
        select_parts.join(", ")
    );

    let mut stmt = conn
        .prepare(&sql)
        .unwrap_or_else(|err| fail(format!("prepare query failed: {err}")));
    let mut rows = stmt
        .query([])
        .unwrap_or_else(|err| fail(format!("query failed: {err}")));

    while let Ok(Some(row)) = rows.next() {
        let id = row.get_ref(0).map(value_to_string).unwrap_or_default();
        let name = row.get_ref(1).map(value_to_string).unwrap_or_default();
        let parent_id = row.get_ref(2).map(value_to_string).unwrap_or_default();
        let sort_order = row.get_ref(3).map(value_to_string).unwrap_or_default();
        let attribute = row.get_ref(4).map(value_to_string).unwrap_or_default();
        println!(
            "playlist|db=edb|id={}|name={}|parent_id={}|sort_order={}|attribute={}",
            id, name, parent_id, sort_order, attribute
        );
    }
}

fn list_pdb_playlists(input: &Path) {
    let pdb_path = resolve_pdb_path(input);
    if !pdb_path.is_file() {
        fail(format!("PDB not found: {}", pdb_path.display()));
    }
    let parsed = parse_pdb(&pdb_path)
        .unwrap_or_else(|err| fail(format!("failed to parse {}: {err}", pdb_path.display())));
    for row in parsed.playlist_tree {
        println!(
            "playlist|db=pdb|id={}|name={}|parent_id={}|sort_order={}|is_folder={}",
            row.id, row.name, row.parent_id, row.sort_order, row.row_is_folder
        );
    }
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        usage();
    }

    let db_type = args[1].trim().to_ascii_lowercase();
    let input = PathBuf::from(&args[2]);

    match db_type.as_str() {
        "app" => list_app_playlists(&input),
        "edb" => list_edb_playlists(&input),
        "pdb" => list_pdb_playlists(&input),
        _ => usage(),
    }
}
