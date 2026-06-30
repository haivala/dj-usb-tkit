use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::{AnalyzeNewTracksRequest, GetPlaylistTracksRequest};

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        fail("usage: cargo run --bin analyze_playlist_tracks -- <data_dir> <playlist_id>");
    }

    let data_dir = PathBuf::from(&args[1]);
    let playlist_id = args[2].trim().to_string();
    if playlist_id.is_empty() {
        fail("playlist_id must not be empty");
    }

    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));

    let tracks_resp = backend.get_playlist_tracks(GetPlaylistTracksRequest {
        playlist_id: playlist_id.clone(),
    });
    if !tracks_resp.ok {
        fail(format!(
            "get_playlist_tracks failed: {}",
            tracks_resp
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }
    let tracks_data = tracks_resp
        .data
        .unwrap_or_else(|| fail("missing get_playlist_tracks data"));
    let track_ids = tracks_data
        .items
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();
    if track_ids.is_empty() {
        fail("playlist has no tracks");
    }

    let analyze_resp = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids,
        analysis_engine: None,
    });
    if !analyze_resp.ok {
        fail(format!(
            "analyze_new_tracks failed: {}",
            analyze_resp
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }
    let analyze_data = analyze_resp
        .data
        .unwrap_or_else(|| fail("missing analyze_new_tracks data"));

    println!(
        "analyzed playlist '{}' tracks: analyzed={} failed={}",
        playlist_id, analyze_data.analyzed, analyze_data.failed
    );
    if !analyze_data.warnings.is_empty() {
        for warning in analyze_data.warnings {
            println!("warning: {warning}");
        }
    }
}
