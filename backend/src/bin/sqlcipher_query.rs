use std::env;
use std::path::PathBuf;

use rusqlite::{Connection, types::ValueRef};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        eprintln!("usage: cargo run --bin sqlcipher_query -- <db_path> <sql>");
        std::process::exit(2);
    }
    let db_path = PathBuf::from(&args[1]);
    let sql = args[2..].join(" ");

    let conn = match open_with_known_keys(&db_path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to open {}: {err}", db_path.display());
            std::process::exit(1);
        }
    };

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("query prepare failed: {err}");
            std::process::exit(1);
        }
    };

    let col_count = stmt.column_count();
    let col_names = stmt
        .column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    println!("{}", col_names.join("|"));

    let mut rows = match stmt.query([]) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("query execution failed: {err}");
            std::process::exit(1);
        }
    };
    while let Ok(Some(row)) = rows.next() {
        let mut out = Vec::<String>::new();
        for idx in 0..col_count {
            let s = match row.get_ref(idx) {
                Ok(ValueRef::Null) => "NULL".to_string(),
                Ok(ValueRef::Integer(v)) => v.to_string(),
                Ok(ValueRef::Real(v)) => v.to_string(),
                Ok(ValueRef::Text(v)) => String::from_utf8_lossy(v).to_string(),
                Ok(ValueRef::Blob(v)) => format!("<blob:{}>", v.len()),
                Err(err) => format!("<err:{err}>"),
            };
            out.push(s);
        }
        println!("{}", out.join("|"));
    }
}

fn open_with_known_keys(path: &PathBuf) -> Result<Connection, String> {
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
