//! Builds the data behind the docs' cue-editor GIFs (see
//! `vanilla-ui/scripts/doc-gifs/record.mjs`): scans and analyzes a source
//! folder into a fresh data dir, then writes what the frontend would receive
//! (`get_tracks_by_ids_with_previews` for every track, `get_track_detail` for
//! the one opened in the editor) as JSON.
use backend::models::{
    AnalyzeNewTracksRequest, GetTrackDetailRequest, GetTracksByIdsRequest, ListTracksRequest,
    ScanLibraryRequest,
};
use backend::service::BackendService;
use std::env;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 5 {
        eprintln!(
            "usage: cargo run --features dev-tools --bin dump_doc_gif_fixture -- <data_dir> <source_dir> <editor_track_title> <out.json>"
        );
        std::process::exit(2);
    }
    let (data_dir, source_dir, editor_title, out) = (&args[1], &args[2], &args[3], &args[4]);
    let fail = |what: &str, err: backend::error::BackendError| -> ! {
        eprintln!("{what} failed: {err}");
        std::process::exit(1);
    };

    let svc = BackendService::new(data_dir).unwrap_or_else(|e| fail("open backend", e));
    svc.scan_library(ScanLibraryRequest {
        source_roots: vec![source_dir.clone()],
        incremental: false,
    })
    .unwrap_or_else(|e| fail("scan_library", e));
    let ids = svc
        .list_tracks(ListTracksRequest {
            limit: 1000,
            cursor: None,
        })
        .unwrap_or_else(|e| fail("list_tracks", e))
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    svc.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids: ids.clone(),
        ..Default::default()
    })
    .unwrap_or_else(|e| fail("analyze_new_tracks", e));

    let tracks = svc
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest { track_ids: ids })
        .unwrap_or_else(|e| fail("get_tracks_by_ids_with_previews", e))
        .items;
    let Some(editor_track) = tracks.iter().find(|t| &t.title == editor_title) else {
        eprintln!("no track titled {editor_title:?} in {source_dir}");
        std::process::exit(1);
    };
    let detail = svc
        .get_track_detail(GetTrackDetailRequest {
            track_id: editor_track.id.clone(),
        })
        .unwrap_or_else(|e| fail("get_track_detail", e));

    let json = serde_json::json!({ "tracks": tracks, "detail": detail });
    std::fs::write(out, serde_json::to_string(&json).expect("serialize"))
        .unwrap_or_else(|e| fail("write output", e.into()));
}
