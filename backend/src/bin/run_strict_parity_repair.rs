use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::{RepairUsbDiagnosticsRequest, RunUsbParityReportRequest};

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 || args.len() > 4 {
        fail(
            "usage: cargo run --bin run_strict_parity_repair -- <data_dir> <usb_root> [apply|preview]",
        );
    }
    let data_dir = PathBuf::from(&args[1]);
    let usb_root = args[2].clone();
    let mode = args.get(3).map(|s| s.as_str()).unwrap_or("preview");
    let apply = matches!(mode, "apply" | "Apply" | "APPLY");

    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));

    let before = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb_root.clone()),
    });
    if !before.ok {
        fail(format!(
            "parity report failed: {}",
            before
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown")
        ));
    }
    let before_data = before.data.expect("parity data");
    let before_failed = before_data
        .playlist_details
        .iter()
        .filter(|p| !matches!(p.status, backend::models::DiagStatus::Pass))
        .count();
    println!("before.overall={:?}", before_data.overall_status);
    println!("before.failed_playlists={before_failed}");
    let before_failed_checks = before_data
        .checks
        .iter()
        .filter(|c| !matches!(c.status, backend::models::DiagStatus::Pass))
        .map(|c| format!("{}:{:?}", c.label, c.status))
        .collect::<Vec<_>>();
    println!("before.failed_checks={}", before_failed_checks.join(" || "));
    for p in before_data
        .playlist_details
        .iter()
        .filter(|p| !matches!(p.status, backend::models::DiagStatus::Pass))
    {
        println!(
            "before.playlist|name={}|status={:?}|dup_pdb={}|dict_issues={}|pdb_missing={}|edb_missing={}|only_pdb={}|only_edb={}|only_pdb_samples={}|only_edb_samples={}|samples={}",
            p.name,
            p.status,
            p.pdb_duplicate_entries,
            p.dictionary_id_issue_tracks,
            p.pdb_missing_core_metadata,
            p.edb_missing_core_metadata,
            p.only_in_pdb,
            p.only_in_edb,
            p.sample_only_in_pdb.join(" ;; "),
            p.sample_only_in_edb.join(" ;; "),
            p.sample_metadata_mismatches.join(" ;; ")
        );
    }

    let repair_response = backend.repair_usb_diagnostics_with_progress(
        RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_root.clone()),
            apply,
            selected_fix_ids: vec![], // empty = run all supported fixes
        },
        |current, total, message| {
            println!("repair.progress={current}/{total}|{message}");
        },
    );
    if !repair_response.ok {
        fail(
            repair_response
                .error
                .as_ref()
                .map(|e| format!("repair failed: {}", e.message))
                .unwrap_or_else(|| "repair failed".to_string()),
        );
    }
    let repair_data = repair_response
        .data
        .unwrap_or_else(|| fail("missing repair response data"));
    println!("repair.apply={apply}");
    println!("repair.applied={}", repair_data.applied_fixes.join(" || "));
    println!("repair.failed={}", repair_data.failed_fixes.join(" || "));
    println!("repair.skipped={}", repair_data.skipped_fixes.join(" || "));
    for w in &repair_data.warnings {
        println!(
            "repair.warning|level={}|code={}|{}",
            w.level, w.code, w.message
        );
    }

    let after = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb_root.clone()),
    });
    if !after.ok {
        fail(format!(
            "parity report after failed: {}",
            after
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown")
        ));
    }
    let after_data = after.data.expect("parity after data");
    let after_failed = after_data
        .playlist_details
        .iter()
        .filter(|p| !matches!(p.status, backend::models::DiagStatus::Pass))
        .count();
    println!("after.overall={:?}", after_data.overall_status);
    println!("after.failed_playlists={after_failed}");
    let after_failed_checks = after_data
        .checks
        .iter()
        .filter(|c| !matches!(c.status, backend::models::DiagStatus::Pass))
        .map(|c| format!("{}:{:?}", c.label, c.status))
        .collect::<Vec<_>>();
    println!("after.failed_checks={}", after_failed_checks.join(" || "));
    for p in after_data
        .playlist_details
        .iter()
        .filter(|p| !matches!(p.status, backend::models::DiagStatus::Pass))
    {
        println!(
            "after.playlist|name={}|status={:?}|dup_pdb={}|dict_issues={}|pdb_missing={}|edb_missing={}|only_pdb={}|only_edb={}|only_pdb_samples={}|only_edb_samples={}|samples={}",
            p.name,
            p.status,
            p.pdb_duplicate_entries,
            p.dictionary_id_issue_tracks,
            p.pdb_missing_core_metadata,
            p.edb_missing_core_metadata,
            p.only_in_pdb,
            p.only_in_edb,
            p.sample_only_in_pdb.join(" ;; "),
            p.sample_only_in_edb.join(" ;; "),
            p.sample_metadata_mismatches.join(" ;; ")
        );
    }
}
