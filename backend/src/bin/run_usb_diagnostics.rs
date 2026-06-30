use backend::commands::BackendCommands;
use backend::models::RunUsbDiagnosticsRequest;
use serde_json::json;
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        eprintln!("usage: run_usb_diagnostics <usb_root> [data_dir]");
        std::process::exit(2);
    }

    let usb_root = args.remove(0);
    let data_dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".tmp-diag-data"));

    let backend = match BackendCommands::new(&data_dir) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to init backend: {err:?}");
            std::process::exit(1);
        }
    };

    let response = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb_root),
    });

    if !response.ok {
        eprintln!("diagnostics failed: {:?}", response.error);
        std::process::exit(1);
    }

    let Some(data) = response.data else {
        eprintln!("diagnostics returned no data");
        std::process::exit(1);
    };

    let snapshot = data.cdj_counter_snapshot.clone().map(|s| {
        json!({
            "playlistCountCandidate": s.playlist_count_candidate,
            "songCountCandidate": s.song_count_candidate,
            "shapeMode": s.shape_mode,
            "baselineInitLike": s.baseline_init_like,
            "t11": s.t11,
            "t12": s.t12,
            "t17": s.t17,
            "t18": s.t18,
            "t19": s.t19,
        })
    });

    let payload = json!({
        "overallStatus": data.overall_status,
        "warnings": data.warnings,
        "cdjCounterSnapshot": snapshot,
        "playlistDetailCount": data.playlist_details.len(),
    });
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
}
