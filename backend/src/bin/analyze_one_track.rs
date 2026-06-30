use backend::commands::BackendCommands;
use backend::models::{AnalyzeNewTracksRequest, SearchTracksRequest};
use std::env;
use std::path::PathBuf;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        eprintln!(
            "usage: cargo run --bin analyze_one_track -- <data_dir> <track_id> [stratum|essentia]"
        );
        std::process::exit(2);
    }
    let data_dir = PathBuf::from(&args[1]);
    let track_id = args[2].trim().to_string();
    if track_id.is_empty() {
        eprintln!("track_id must not be empty");
        std::process::exit(2);
    }
    let analysis_engine = args.get(3).map(|v| v.trim().to_ascii_lowercase());
    let analysis_engine = match analysis_engine.as_deref() {
        Some("") | None => None,
        Some("stratum") => Some("stratum".to_string()),
        Some("essentia") => Some("essentia".to_string()),
        Some(other) => {
            eprintln!("invalid engine '{}'; expected stratum or essentia", other);
            std::process::exit(2);
        }
    };

    let backend = match BackendCommands::new(&data_dir) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to open backend: {}", err.message);
            std::process::exit(1);
        }
    };

    let analyzed = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids: vec![track_id.clone()],
        analysis_engine,
    });
    if !analyzed.ok {
        let msg = analyzed
            .error
            .as_ref()
            .map(|e| e.message.as_str())
            .unwrap_or("unknown analyze error");
        eprintln!("analyze_new_tracks failed: {msg}");
        std::process::exit(1);
    }
    let analyzed_data = analyzed.data.expect("analysis data");
    println!(
        "analyze result: analyzed={} failed={}",
        analyzed_data.analyzed, analyzed_data.failed
    );
    if !analyzed_data.warnings.is_empty() {
        println!("warnings:");
        for warning in analyzed_data.warnings {
            println!("  - {warning}");
        }
    }

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 1000,
        cursor: None,
    });
    if !search.ok {
        let msg = search
            .error
            .as_ref()
            .map(|e| e.message.as_str())
            .unwrap_or("unknown search error");
        eprintln!("search_tracks failed: {msg}");
        std::process::exit(1);
    }
    let items = search.data.expect("search data").items;
    if let Some(track) = items.into_iter().find(|t| t.id == track_id) {
        println!(
            "track: bpm={:?} key={:?} waveform={} artwork={}",
            track.bpm,
            track.key,
            track
                .waveform_peaks_path
                .as_ref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false),
            track
                .artwork_path
                .as_ref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        );
    } else {
        eprintln!("track not found after analysis: {}", track_id);
        std::process::exit(1);
    }
}
