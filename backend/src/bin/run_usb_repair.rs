/// Run all USB repair fixes (or a preview) on a USB root.
///
/// Usage:
///   cargo run --manifest-path backend/Cargo.toml --bin run_usb_repair -- \
///       <usb_root> [data_dir] [apply|preview]
///
/// Default mode is "preview" (dry-run). Pass "apply" to actually write changes.
use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::RepairUsbDiagnosticsRequest;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: run_usb_repair <usb_root> [data_dir] [apply|preview]");
        std::process::exit(2);
    }
    let usb_root = args[1].clone();
    let data_dir = args
        .get(2)
        .filter(|s| !matches!(s.as_str(), "apply" | "preview"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".tmp-diag-data"));
    let mode = args
        .iter()
        .find(|s| matches!(s.as_str(), "apply" | "preview"))
        .map(|s| s.as_str())
        .unwrap_or("preview");
    let apply = mode == "apply";

    let backend = match BackendCommands::new(&data_dir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to init backend: {:?}", e);
            std::process::exit(1);
        }
    };

    println!("mode={mode}  usb={usb_root}");
    println!();

    let resp = backend.repair_usb_diagnostics_with_progress(
        RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_root.clone()),
            apply,
            selected_fix_ids: vec![], // empty = select all
        },
        |cur, tot, msg| eprintln!("  [{cur}/{tot}] {msg}"),
    );

    if !resp.ok {
        eprintln!(
            "repair failed: {}",
            resp.error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown")
        );
        std::process::exit(1);
    }

    let data = resp.data.unwrap();

    println!("detected_issues ({}):", data.detected_issues.len());
    for s in &data.detected_issues {
        println!("  {s}");
    }
    println!();
    println!("proposed_fixes ({}):", data.proposed_fixes.len());
    for f in &data.proposed_fixes {
        println!("  [{}] {}", f.id, f.title);
    }
    println!();
    if apply {
        println!("applied ({}):", data.applied_fixes.len());
        for s in &data.applied_fixes {
            println!("  {s}");
        }
        println!("skipped ({}):", data.skipped_fixes.len());
        for s in &data.skipped_fixes {
            println!("  {s}");
        }
        if !data.failed_fixes.is_empty() {
            println!("FAILED ({}):", data.failed_fixes.len());
            for s in &data.failed_fixes {
                println!("  {s}");
            }
        }
    }
    println!();
    let errors: Vec<_> = data
        .warnings
        .iter()
        .filter(|w| w.level == "error")
        .collect();
    if !errors.is_empty() {
        println!("warnings/errors ({}):", errors.len());
        for w in &errors {
            println!("  [{}] {}", w.code, w.message);
        }
    } else if apply {
        println!("no error-level warnings after repair");
    }
}
