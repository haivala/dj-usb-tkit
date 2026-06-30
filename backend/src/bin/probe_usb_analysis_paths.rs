use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::{FetchUsbPlaylistsRequest, InspectUsbTrackRequest};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackProbeRow {
    id: String,
    title: String,
    artist: String,
    album: Option<String>,
    track_number: Option<u32>,
    bpm: Option<f64>,
    key: Option<String>,
    file_path: String,
    usb_media_path: Option<String>,
    usb_analysis_path: Option<String>,
    usb_analysis_path_raw: Option<String>,
    playlist_ids: Vec<String>,
    playlist_names: Vec<String>,
}

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        fail(
            "usage: cargo run --bin probe_usb_analysis_paths -- <data_dir> <usb_root> [track_id ...]",
        );
    }

    let data_dir = PathBuf::from(&args[1]);
    let usb_root = PathBuf::from(&args[2]);
    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));

    if args.len() > 3 {
        let mut rows = Vec::<TrackProbeRow>::new();
        for track_id in &args[3..] {
            let resp = backend.inspect_usb_track(InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: track_id.clone(),
                file_path: None,
                title: None,
                artist: None,
            });
            if !resp.ok {
                fail(
                    resp.error
                        .as_ref()
                        .map(|e| format!("inspect_usb_track({track_id}) failed: {}", e.message))
                        .unwrap_or_else(|| format!("inspect_usb_track({track_id}) failed")),
                );
            }
            let data = resp
                .data
                .unwrap_or_else(|| fail("missing inspect_usb_track response data"));
            let track = data.track;
            rows.push(TrackProbeRow {
                id: track.id,
                title: track.title,
                artist: track.artist,
                album: track.album,
                track_number: track.track_number,
                bpm: track.bpm,
                key: track.key,
                file_path: track.file_path,
                usb_media_path: track.usb_media_path,
                usb_analysis_path: track.usb_analysis_path,
                usb_analysis_path_raw: track.usb_analysis_path_raw,
                playlist_ids: Vec::new(),
                playlist_names: Vec::new(),
            });
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).expect("serialize inspect rows")
        );
        return;
    }

    let resp = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb_root.to_string_lossy().to_string()),
    });
    if !resp.ok {
        fail(
            resp.error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "fetch_usb_playlists failed".to_string()),
        );
    }

    let data = resp
        .data
        .unwrap_or_else(|| fail("missing fetch_usb_playlists response data"));

    let mut tracks = BTreeMap::<String, TrackProbeRow>::new();
    for playlist in data.items {
        for track in playlist.tracks {
            let key = track
                .usb_media_path
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| track.file_path.clone());
            let entry = tracks.entry(key).or_insert_with(|| TrackProbeRow {
                id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                track_number: track.track_number,
                bpm: track.bpm,
                key: track.key.clone(),
                file_path: track.file_path.clone(),
                usb_media_path: track.usb_media_path.clone(),
                usb_analysis_path: track.usb_analysis_path.clone(),
                usb_analysis_path_raw: track.usb_analysis_path_raw.clone(),
                playlist_ids: Vec::new(),
                playlist_names: Vec::new(),
            });
            if !entry.playlist_ids.iter().any(|v| v == &playlist.id) {
                entry.playlist_ids.push(playlist.id.clone());
            }
            if !entry.playlist_names.iter().any(|v| v == &playlist.name) {
                entry.playlist_names.push(playlist.name.clone());
            }
        }
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&tracks.into_values().collect::<Vec<_>>())
            .expect("serialize probe rows")
    );
}
