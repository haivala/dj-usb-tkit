use std::env;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use backend::shape_compare::compare_usb_shape;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    let summary = args.iter().any(|a| a == "--summary");
    let positional: Vec<&str> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();
    if positional.len() != 2 {
        eprintln!(
            "usage: cargo run --bin compare_usb_shape -- [--summary] <usb_expected_root> <usb_actual_root>"
        );
        eprintln!("  --summary  print human-readable diff instead of full JSON");
        std::process::exit(2);
    }

    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let run = panic::catch_unwind(AssertUnwindSafe(|| {
        compare_usb_shape(Path::new(positional[0]), Path::new(positional[1]))
    }));
    panic::set_hook(prev_hook);

    let diff = match run {
        Ok(diff) => diff,
        Err(err) => {
            let msg = if let Some(s) = err.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = err.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("compare_usb_shape failed: {msg}");
            std::process::exit(1);
        }
    };

    if summary {
        println!("strict_match: {}", diff.strict_match);
        println!("pdb_len_page_match: {}", diff.pdb_len_page_match);
        println!("pdb_num_tables_match: {}", diff.pdb_num_tables_match);
        if !diff.missing_pdb_table_ids.is_empty() {
            println!("missing_pdb_table_ids: {:?}", diff.missing_pdb_table_ids);
        }
        if !diff.extra_pdb_table_ids.is_empty() {
            println!("extra_pdb_table_ids: {:?}", diff.extra_pdb_table_ids);
        }
        if !diff.pdb_pointer_diffs.is_empty() {
            println!("pdb_pointer_diffs ({}):", diff.pdb_pointer_diffs.len());
            for d in &diff.pdb_pointer_diffs {
                println!(
                    "  tt={} first: {}=={} last: {}!={}",
                    d.table_type,
                    d.expected_first_page,
                    d.actual_first_page,
                    d.expected_last_page,
                    d.actual_last_page
                );
            }
        }
        if !diff.pdb_table_shape_diffs.is_empty() {
            println!(
                "pdb_table_shape_diffs ({}):",
                diff.pdb_table_shape_diffs.len()
            );
            for d in &diff.pdb_table_shape_diffs {
                let e = &d.expected;
                let a = &d.actual;
                println!(
                    "  tt={}: pages {}!={} rows_pages {}!={} empty_pages {}!={}",
                    d.table_type,
                    e.pages,
                    a.pages,
                    e.rows_pages,
                    a.rows_pages,
                    e.empty_pages,
                    a.empty_pages
                );
            }
        }
        if !diff.missing_edb_tables.is_empty() {
            println!("missing_edb_tables: {:?}", diff.missing_edb_tables);
        }
        if !diff.extra_edb_tables.is_empty() {
            println!("extra_edb_tables: {:?}", diff.extra_edb_tables);
        }
        if !diff.edb_schema_table_diffs.is_empty() {
            println!(
                "edb_schema_table_diffs ({}):",
                diff.edb_schema_table_diffs.len()
            );
            for d in &diff.edb_schema_table_diffs {
                if !d.missing_columns.is_empty() {
                    println!("  {}: missing {:?}", d.table, d.missing_columns);
                }
                if !d.extra_columns.is_empty() {
                    println!("  {}: extra {:?}", d.table, d.extra_columns);
                }
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&diff).expect("serialize shape diff")
        );
    }
    if !diff.strict_match {
        std::process::exit(1);
    }
}
