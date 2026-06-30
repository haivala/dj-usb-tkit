use std::env;
use std::path::PathBuf;
use std::time::Instant;

use backend::commands::BackendCommands;
use backend::models::{AnalyzeNewTracksRequest, ScanLibraryRequest, SearchTracksRequest};

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!(
            "warning: benchmark running in debug profile; use --release for meaningful timing"
        );
    }

    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 || args.len() > 4 {
        fail(
            "usage: cargo run --manifest-path backend/Cargo.toml --bin benchmark_analyze_tracks -- <data_dir> <source_root> [runs]",
        );
    }

    let data_dir = PathBuf::from(&args[1]);
    let source_root = args[2].trim().to_string();
    if source_root.is_empty() {
        fail("source_root must not be empty");
    }
    let runs = if args.len() == 4 {
        match args[3].parse::<usize>() {
            Ok(v) if v >= 1 => v,
            Ok(_) => fail("runs must be >= 1"),
            Err(_) => fail("runs must be a positive integer"),
        }
    } else {
        1
    };

    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));

    let workers = backend
        .get_system_parallelism()
        .data
        .map(|d| d.workers)
        .unwrap_or(1);

    let scan_started = Instant::now();
    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_root.clone()],
        incremental: true,
    });
    if !scan.ok {
        fail(format!(
            "scan_library failed: {}",
            scan.error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }
    let scan_elapsed = scan_started.elapsed();

    let mut all_track_ids = Vec::<String>::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = backend.search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 500,
            cursor,
        });
        if !page.ok {
            fail(format!(
                "search_tracks failed: {}",
                page.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ));
        }
        let data = page
            .data
            .unwrap_or_else(|| fail("missing search_tracks data"));
        all_track_ids.extend(data.items.into_iter().map(|t| t.id));
        if !data.has_more {
            break;
        }
        cursor = data.next_cursor;
    }

    if all_track_ids.is_empty() {
        fail("no tracks found after scan");
    }

    println!(
        "scan complete: source_root='{}' tracks={} workers={} scan_s={:.3}",
        source_root,
        all_track_ids.len(),
        workers,
        scan_elapsed.as_secs_f64()
    );

    let mut elapsed = Vec::<f64>::with_capacity(runs);
    for run in 1..=runs {
        let started = Instant::now();
        let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
            track_ids: all_track_ids.clone(),
            analysis_engine: None,
        });
        let seconds = started.elapsed().as_secs_f64();
        if !analyze.ok {
            fail(format!(
                "analyze_new_tracks failed on run {}: {}",
                run,
                analyze
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ));
        }
        let data = analyze
            .data
            .unwrap_or_else(|| fail("missing analyze_new_tracks data"));
        let tps = if seconds > 0.0 {
            data.analyzed as f64 / seconds
        } else {
            0.0
        };
        println!(
            "run={} analyzed={} failed={} warnings={} seconds={:.3} tracks_per_sec={:.2}",
            run,
            data.analyzed,
            data.failed,
            data.warnings.len(),
            seconds,
            tps
        );
        elapsed.push(seconds);
    }

    let total: f64 = elapsed.iter().sum();
    let avg = total / elapsed.len() as f64;
    let best = elapsed.iter().copied().fold(f64::INFINITY, f64::min);
    let worst = elapsed.iter().copied().fold(0.0, f64::max);
    println!(
        "summary runs={} avg_s={:.3} best_s={:.3} worst_s={:.3}",
        runs, avg, best, worst
    );
}
