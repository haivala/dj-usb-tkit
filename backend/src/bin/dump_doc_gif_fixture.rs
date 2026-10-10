//! Builds the data behind the docs' GIFs and screenshots (see
//! `vanilla-ui/scripts/doc-media/record.mjs`): scans and analyzes a source
//! folder into a fresh data dir, then writes what the frontend would receive
//! (`get_tracks_by_ids_with_previews` for every track, `get_track_detail` for
//! the one opened in the editor) as JSON.
//!
//! With a `<usb_spec.json>` (`{ usbRoot, usbName, brokenUsbRoot, playlists:
//! [{ name, titles }] }`) it also names the USB `usbName` and exports those
//! playlists to `usbRoot`, in order (recording each one's playlist id), and
//! adds the USB views' responses under `usb`: the USB playlists and their
//! tracks, history, player menu and diagnostics, each export's progress
//! messages and result, and -- for a copy of the USB at `brokenUsbRoot` with
//! one audio file renamed, one analysis file emptied and one playlist entry
//! dropped from the eDB only -- the diagnostics and the repair preview.
use backend::models::{
    AnalyzeNewTracksRequest, ApiResponse, GetTrackDetailRequest, GetTracksByIdsRequest,
    ListTracksRequest, ScanLibraryRequest,
};
use backend::service::BackendService;
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn fail(what: &str, err: impl std::fmt::Display) -> ! {
    eprintln!("{what} failed: {err}");
    std::process::exit(1);
}

/// A request from its camelCase JSON, as the frontend sends it.
fn request<T: DeserializeOwned>(what: &str, value: Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|e| fail(what, e))
}

/// The response the frontend would receive for `result`.
fn response<T: Serialize>(what: &str, result: backend::error::BackendResult<T>) -> Value {
    let data = result.unwrap_or_else(|e| fail(what, e));
    serde_json::to_value(ApiResponse::success(data)).expect("serialize")
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 5 {
        eprintln!(
            "usage: cargo run --features dev-tools --bin dump_doc_gif_fixture -- <data_dir> <source_dir> <editor_track_title> <out.json> [usb_spec.json]"
        );
        std::process::exit(2);
    }
    let (data_dir, source_dir, editor_title, out) = (&args[1], &args[2], &args[3], &args[4]);

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

    let mut json = json!({ "tracks": tracks, "detail": detail });
    if let Some(spec) = args.get(5) {
        let spec: Value = serde_json::from_str(
            &fs::read_to_string(spec).unwrap_or_else(|e| fail("read usb spec", e)),
        )
        .unwrap_or_else(|e| fail("parse usb spec", e));
        let id_of = |title: &str| {
            tracks
                .iter()
                .find(|t| t.title == title)
                .map(|t| t.id.clone())
                .unwrap_or_else(|| fail("usb spec", format!("no track titled {title:?}")))
        };
        json["usb"] = dump_usb(&svc, &spec, id_of);
    }
    fs::write(out, serde_json::to_string(&json).expect("serialize"))
        .unwrap_or_else(|e| fail("write output", e));
}

fn dump_usb(svc: &BackendService, spec: &Value, id_of: impl Fn(&str) -> String) -> Value {
    let usb_root = spec["usbRoot"].as_str().expect("usbRoot");
    let broken_root = spec["brokenUsbRoot"].as_str().expect("brokenUsbRoot");
    fs::create_dir_all(usb_root).unwrap_or_else(|e| fail("create usb root", e));
    svc.initialize_usb(request("initialize_usb", json!({ "usbRoot": usb_root })))
        .unwrap_or_else(|e| fail("initialize_usb", e));
    svc.set_usb_device_name(request(
        "set_usb_device_name",
        json!({ "usbRoot": usb_root, "name": spec["usbName"] }),
    ))
    .unwrap_or_else(|e| fail("set_usb_device_name", e));

    let mut exports = serde_json::Map::new();
    for playlist in spec["playlists"].as_array().expect("playlists") {
        let name = playlist["name"].as_str().expect("playlist name");
        let created = svc
            .create_playlist(request("create_playlist", json!({ "name": name })))
            .unwrap_or_else(|e| fail("create_playlist", e));
        let track_ids = playlist["titles"]
            .as_array()
            .expect("titles")
            .iter()
            .map(|t| id_of(t.as_str().expect("title")))
            .collect::<Vec<_>>();
        svc.add_tracks_to_playlist(request(
            "add_tracks_to_playlist",
            json!({ "playlistId": created.playlist_id, "trackIds": track_ids, "dedupe": "skip" }),
        ))
        .unwrap_or_else(|e| fail("add_tracks_to_playlist", e));
        let mut progress = Vec::new();
        let result = svc.export_to_usb_with_progress(
            request(
                "export_to_usb",
                json!({
                    "usbRoot": usb_root,
                    "playlistId": created.playlist_id,
                    "options": {
                        "includeArtwork": true,
                        "includeAnalysis": true,
                        "pruneStale": true,
                        "backupBeforeExport": false,
                    },
                }),
            ),
            |current, total, message| progress.push(json!([current, total, message])),
        );
        exports.insert(
            name.to_string(),
            json!({
                "playlistId": created.playlist_id,
                "progress": progress,
                "response": response("export_to_usb", result),
            }),
        );
    }

    let root = json!({ "usbRoot": usb_root });
    let playlists = response(
        "fetch_usb_playlists",
        svc.fetch_usb_playlists(request("fetch_usb_playlists", root.clone())),
    );
    let mut playlist_tracks = serde_json::Map::new();
    for item in playlists["data"]["items"]
        .as_array()
        .expect("usb playlists")
    {
        let id = item["id"].as_str().expect("usb playlist id");
        let tracks = svc.fetch_usb_playlist_tracks(request(
            "fetch_usb_playlist_tracks",
            json!({ "usbRoot": usb_root, "id": id }),
        ));
        playlist_tracks.insert(
            id.to_string(),
            response("fetch_usb_playlist_tracks", tracks),
        );
    }
    let histories = response(
        "fetch_usb_histories",
        svc.fetch_usb_histories(request("fetch_usb_histories", root.clone())),
    );
    let player_menu = response(
        "get_usb_player_menu_config",
        svc.get_usb_player_menu_config(request("get_usb_player_menu_config", root.clone())),
    );
    let diagnostics = response(
        "run_usb_diagnostics",
        svc.run_usb_diagnostics(request("run_usb_diagnostics", root)),
    );

    break_usb_copy(Path::new(usb_root), Path::new(broken_root));
    let broken = json!({ "usbRoot": broken_root });
    let broken_diagnostics = response(
        "run_usb_diagnostics (broken)",
        svc.run_usb_diagnostics(request("run_usb_diagnostics", broken.clone())),
    );
    let repair_preview = response(
        "repair_usb_diagnostics (broken)",
        svc.repair_usb_diagnostics(request("repair_usb_diagnostics", broken)),
    );

    json!({
        "exports": exports,
        "playlists": playlists,
        "playlistTracks": playlist_tracks,
        "histories": histories,
        "playerMenu": player_menu,
        "diagnostics": diagnostics,
        "brokenDiagnostics": broken_diagnostics,
        "repairPreview": repair_preview,
    })
}

/// Copies the USB to `broken`, then renames its first audio file (as if
/// renamed by hand), empties the analysis file of another track, and drops
/// one playlist entry and its track from the eDB but not the PDB (so they
/// disagree).
fn break_usb_copy(usb: &Path, broken: &Path) {
    let files = |root: &Path, ext: &str| {
        let mut found = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
            .collect::<Vec<PathBuf>>();
        found.sort();
        found
    };
    for entry in walkdir::WalkDir::new(usb)
        .into_iter()
        .filter_map(Result::ok)
    {
        let target = broken.join(entry.path().strip_prefix(usb).expect("under usb"));
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).unwrap_or_else(|e| fail("copy usb", e));
        } else {
            fs::copy(entry.path(), &target).unwrap_or_else(|e| fail("copy usb", e));
        }
    }
    let audio = files(&broken.join("Contents"), "mp3");
    let renamed = audio
        .first()
        .unwrap_or_else(|| fail("break usb", "no audio"));
    let stem = renamed.file_stem().expect("stem").to_string_lossy();
    fs::rename(
        renamed,
        renamed.with_file_name(format!("{stem} (edit).mp3")),
    )
    .unwrap_or_else(|e| fail("rename audio", e));
    let analysis = files(&broken.join("PIONEER/USBANLZ"), "dat");
    let emptied = analysis
        .last()
        .unwrap_or_else(|| fail("break usb", "no analysis"));
    fs::write(emptied, []).unwrap_or_else(|e| fail("empty analysis", e));

    let edb = backend::edb::edb_path_from_usb_root(broken);
    let conn = rusqlite::Connection::open(&edb).unwrap_or_else(|e| fail("open eDB", e));
    conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
        .unwrap_or_else(|e| fail("unlock eDB", e));
    let content_id: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE rowid = (SELECT MAX(rowid) FROM playlist_content)",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| fail("find eDB playlist entry", e));
    let dropped = conn
        .execute(
            "DELETE FROM playlist_content WHERE rowid = (SELECT MAX(rowid) FROM playlist_content)",
            [],
        )
        .unwrap_or_else(|e| fail("drop eDB playlist entry", e));
    if dropped != 1 {
        fail("drop eDB playlist entry", format!("{dropped} rows deleted"));
    }
    // ...and that track's own row, so the PDB has a track the eDB lacks
    // (the diagnostics summary warns for newer players).
    let dropped = conn
        .execute("DELETE FROM content WHERE content_id = ?1", [content_id])
        .unwrap_or_else(|e| fail("drop eDB track", e));
    if dropped != 1 {
        fail("drop eDB track", format!("{dropped} rows deleted"));
    }
}
