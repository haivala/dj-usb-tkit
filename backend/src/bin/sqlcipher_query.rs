use std::env;
use std::path::PathBuf;

use rusqlite::types::ValueRef;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        eprintln!("usage: cargo run --bin sqlcipher_query -- <db_path> <sql>");
        std::process::exit(2);
    }
    let db_path = PathBuf::from(&args[1]);
    let sql = args[2..].join(" ");

    let conn = match backend::edb::open_rekordbox_db_for_tools(&db_path) {
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
