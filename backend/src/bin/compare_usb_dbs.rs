use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, types::Value};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!(
            "usage: cargo run --bin compare_usb_dbs -- <usb_expected_root> <usb_actual_root>"
        );
        std::process::exit(2);
    }

    let expected_db = db_path_from_usb_root(Path::new(&args[1]));
    let actual_db = db_path_from_usb_root(Path::new(&args[2]));

    let expected = match open_export_db(&expected_db) {
        Ok(c) => c,
        Err(err) => {
            eprintln!(
                "failed to open expected db {}: {err}",
                expected_db.display()
            );
            std::process::exit(1);
        }
    };
    let actual = match open_export_db(&actual_db) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to open actual db {}: {err}", actual_db.display());
            std::process::exit(1);
        }
    };

    let mut tables = BTreeSet::<String>::new();
    for t in list_tables(&expected) {
        tables.insert(t);
    }
    for t in list_tables(&actual) {
        tables.insert(t);
    }

    let target_tables = [
        "playlist",
        "content",
        "playlist_content",
        "artist",
        "album",
        "image",
        "key",
    ];

    println!("DB compare (column-level):");
    println!("expected: {}", expected_db.display());
    println!("actual:   {}", actual_db.display());

    for table in target_tables {
        if !tables.contains(table) {
            continue;
        }
        compare_table(&expected, &actual, table);
    }
}

fn db_path_from_usb_root(root: &Path) -> PathBuf {
    let vendor_root = ["PIO", "NEER"].join("");
    let db_dir = ["reko", "rdbox"].join("");
    root.join(vendor_root).join(db_dir).join("exportLibrary.db")
}

fn open_export_db(path: &Path) -> Result<Connection, String> {
    let open = || Connection::open(path).map_err(|e| e.to_string());
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

    let plain = open()?;
    if has_schema(&plain) {
        return Ok(plain);
    }

    let mut keys = vec![
        DEFAULT_USB_EXPORT_KEY.to_string(),
        DEFAULT_MASTER_KEY.to_string(),
    ];
    if let Ok(extra) = env::var("USB_DB_KEY") {
        let t = extra.trim();
        if !t.is_empty() && !keys.iter().any(|k| k == t) {
            keys.insert(0, t.to_string());
        }
    }

    for key in keys {
        let conn = open()?;
        if conn.execute_batch(&format!("PRAGMA key='{key}';")).is_err() {
            continue;
        }
        if has_schema(&conn) {
            return Ok(conn);
        }
    }
    Err("not readable as plain sqlite or with known SQLCipher keys".to_string())
}

fn list_tables(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    out
}

fn compare_table(expected: &Connection, actual: &Connection, table: &str) {
    let expected_cols = table_columns(expected, table);
    let actual_cols = table_columns(actual, table);

    let expected_count = table_count(expected, table);
    let actual_count = table_count(actual, table);

    let common_cols = expected_cols
        .intersection(&actual_cols)
        .cloned()
        .collect::<Vec<_>>();
    let expected_only_cols = expected_cols
        .difference(&actual_cols)
        .cloned()
        .collect::<Vec<_>>();
    let actual_only_cols = actual_cols
        .difference(&expected_cols)
        .cloned()
        .collect::<Vec<_>>();

    println!("\n[{}]", table);
    println!(
        "  rows: expected={} actual={}",
        expected_count, actual_count
    );
    println!(
        "  columns: expected={} actual={} common={}",
        expected_cols.len(),
        actual_cols.len(),
        common_cols.len()
    );
    if !expected_only_cols.is_empty() {
        println!("  expected-only columns: {}", expected_only_cols.join(", "));
    }
    if !actual_only_cols.is_empty() {
        println!("  actual-only columns: {}", actual_only_cols.join(", "));
    }

    if common_cols.is_empty() {
        println!("  no common columns; skipped");
        return;
    }

    let key_cols = key_columns(table);
    let usable_key_cols = key_cols
        .iter()
        .filter(|c| common_cols.iter().any(|cc| cc == *c))
        .cloned()
        .collect::<Vec<_>>();
    if usable_key_cols.is_empty() {
        println!("  no usable key columns for row-level compare; skipped");
        return;
    }

    let expected_rows = load_rows(expected, table, &common_cols, &usable_key_cols);
    let actual_rows = load_rows(actual, table, &common_cols, &usable_key_cols);

    let expected_keys = expected_rows.keys().cloned().collect::<BTreeSet<_>>();
    let actual_keys = actual_rows.keys().cloned().collect::<BTreeSet<_>>();
    let shared_keys = expected_keys
        .intersection(&actual_keys)
        .cloned()
        .collect::<Vec<_>>();

    println!(
        "  keys: shared={} only_expected={} only_actual={}",
        shared_keys.len(),
        expected_keys.difference(&actual_keys).count(),
        actual_keys.difference(&expected_keys).count()
    );

    let mut mismatch_counts = BTreeMap::<String, usize>::new();
    let mut ignored_mismatch_counts = BTreeMap::<String, usize>::new();
    for key in shared_keys {
        let e = expected_rows.get(&key).expect("expected row");
        let a = actual_rows.get(&key).expect("actual row");
        for col in &common_cols {
            let ev = e.get(col).cloned().unwrap_or_default();
            let av = a.get(col).cloned().unwrap_or_default();
            if ev != av {
                if is_allowed_to_differ(table, col) {
                    *ignored_mismatch_counts.entry(col.clone()).or_insert(0) += 1;
                } else {
                    *mismatch_counts.entry(col.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    if mismatch_counts.is_empty() {
        println!("  column mismatches: none");
    } else {
        println!("  column mismatches:");
        for (col, count) in mismatch_counts {
            println!("    {}: {}", col, count);
            if let Some(sample) = sample_mismatch(&expected_rows, &actual_rows, &col) {
                println!(
                    "      sample key={} expected={} actual={}",
                    sample.0, sample.1, sample.2
                );
            }
        }
    }

    if ignored_mismatch_counts.is_empty() {
        println!("  allowed-diff mismatches: none");
    } else {
        println!("  allowed-diff mismatches:");
        for (col, count) in ignored_mismatch_counts {
            println!("    {}: {}", col, count);
            if let Some(sample) = sample_mismatch(&expected_rows, &actual_rows, &col) {
                println!(
                    "      sample key={} expected={} actual={}",
                    sample.0, sample.1, sample.2
                );
            }
        }
    }
}

fn sample_mismatch(
    expected_rows: &HashMap<String, HashMap<String, String>>,
    actual_rows: &HashMap<String, HashMap<String, String>>,
    target_col: &str,
) -> Option<(String, String, String)> {
    let mut keys = expected_rows.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let e = expected_rows.get(&key)?;
        let a = actual_rows.get(&key)?;
        let ev = e.get(target_col)?.clone();
        let av = a.get(target_col)?.clone();
        if ev != av {
            return Some((key, ev, av));
        }
    }
    None
}

fn is_allowed_to_differ(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        (
            "content",
            "analysisDataFilePath" | "contentLink" | "masterContentId" | "masterDbId"
        ) | ("key", "name")
    )
}

fn table_columns(conn: &Connection, table: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::<String>::new();
    let ident = table_ident(table);
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({ident})")) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows = match stmt.query_map([], |r| r.get::<_, String>(1)) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for row in rows.flatten() {
        out.insert(row);
    }
    out
}

fn table_count(conn: &Connection, table: &str) -> i64 {
    let ident = table_ident(table);
    conn.query_row(&format!("SELECT COUNT(1) FROM {ident}"), [], |r| {
        r.get::<_, i64>(0)
    })
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
}

fn key_columns(table: &str) -> Vec<String> {
    match table {
        "content" => vec!["content_id".to_string()],
        "playlist" => vec!["playlist_id".to_string()],
        "artist" => vec!["artist_id".to_string()],
        "album" => vec!["album_id".to_string()],
        "image" => vec!["image_id".to_string()],
        "key" => vec!["key_id".to_string()],
        "playlist_content" => vec![
            "playlist_id".to_string(),
            "content_id".to_string(),
            "sequenceNo".to_string(),
        ],
        _ => vec![],
    }
}

fn load_rows(
    conn: &Connection,
    table: &str,
    columns: &[String],
    key_columns: &[String],
) -> HashMap<String, HashMap<String, String>> {
    let ident = table_ident(table);

    let mut select_cols = key_columns.to_vec();
    for c in columns {
        if !select_cols.iter().any(|x| x == c) {
            select_cols.push(c.clone());
        }
    }
    let select_sql = select_cols
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let mut stmt = match conn.prepare(&format!("SELECT {select_sql} FROM {ident}")) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    let key_index = key_columns
        .iter()
        .filter_map(|kc| select_cols.iter().position(|c| c == kc))
        .collect::<Vec<_>>();

    let mut out = HashMap::<String, HashMap<String, String>>::new();
    let rows = match stmt.query_map([], |row| {
        let mut values = HashMap::<String, String>::new();
        for (idx, col) in select_cols.iter().enumerate() {
            let v = row.get::<usize, Value>(idx)?;
            values.insert(col.clone(), value_to_string(&v));
        }
        let key = key_index
            .iter()
            .map(|idx| values.get(&select_cols[*idx]).cloned().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|");
        Ok((key, values))
    }) {
        Ok(r) => r,
        Err(_) => return HashMap::new(),
    };

    for row in rows.flatten() {
        out.insert(row.0, row.1);
    }
    out
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => format!("{f}"),
        Value::Text(s) => s.to_string(),
        Value::Blob(b) => format!("<blob:{}>", b.len()),
    }
}

fn table_ident(table: &str) -> String {
    if table == "key" {
        "\"key\"".to_string()
    } else {
        table.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::is_allowed_to_differ;

    #[test]
    fn allowed_diff_policy_includes_expected_content_columns() {
        assert!(is_allowed_to_differ("content", "analysisDataFilePath"));
        assert!(is_allowed_to_differ("content", "contentLink"));
        assert!(is_allowed_to_differ("content", "masterContentId"));
        assert!(is_allowed_to_differ("content", "masterDbId"));
    }

    #[test]
    fn allowed_diff_policy_includes_key_name_only_for_key_table() {
        assert!(is_allowed_to_differ("key", "name"));
        assert!(!is_allowed_to_differ("content", "name"));
    }

    #[test]
    fn allowed_diff_policy_rejects_non_allowlisted_columns() {
        assert!(!is_allowed_to_differ("content", "title"));
        assert!(!is_allowed_to_differ("playlist", "name"));
        assert!(!is_allowed_to_differ("image", "path"));
    }
}
