//! Builds the data behind the docs' GIFs and screenshots (see
//! `vanilla-ui/scripts/doc-media/record.mjs`): scans and analyzes a source
//! folder into a fresh data dir, then writes what the frontend would receive
//! (`get_tracks_by_ids_with_previews` for every track, `get_track_detail` for
//! the one opened in the editor, `get_ui_texts` for the labels) as JSON.
//!
//! With a `<usb_spec.json>` (`{ usbRoot, usbName, brokenUsbRoot, playlists:
//! [{ name, titles }] }`) it also names the USB `usbName` and exports those
//! playlists to `usbRoot`, in order (recording each one's playlist id), and
//! adds the USB views' responses under `usb`: the USB playlists and their
//! tracks, history, player menu and diagnostics, each export's progress
//! messages and result, and -- for a copy of the USB at `brokenUsbRoot` with
//! one audio file renamed, one analysis file emptied and one playlist entry
//! dropped from the eDB only -- the diagnostics and the repair preview.
//!
//! `DOC_FIXTURE_LANGUAGE` (e.g. `en-XA`, debug builds only) dumps the responses in that
//! language instead of English, to find response text that isn't from the catalog.
use backend::models::{
    AddSourceFoldersRequest, AnalyzeNewTracksRequest, ApiResponse, BrowseSourceFilesRequest,
    GetTrackDetailRequest, GetTracksByIdsRequest,
};
use backend::service::BackendService;
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn fail(what: &str, err: impl backend::i18n::IntoArg) -> ! {
    eprintln!("{what} failed: {}", backend::i18n::shown(err));
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

    if let Ok(language) = env::var("DOC_FIXTURE_LANGUAGE") {
        backend::i18n::set_language(&language).unwrap_or_else(|e| fail("set_language", e));
    }
    let svc = BackendService::new(data_dir).unwrap_or_else(|e| fail("open backend", e));
    // Adding the folder indexes it.
    svc.add_source_folders(AddSourceFoldersRequest {
        paths: vec![source_dir.clone()],
    })
    .unwrap_or_else(|e| fail("add_source_folders", e));
    let ids = svc
        .browse_source_files(BrowseSourceFilesRequest {
            limit: 1000,
            ..Default::default()
        })
        .unwrap_or_else(|e| fail("browse_source_files", e))
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    svc.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids: ids.clone(),
        ..Default::default()
    })
    .unwrap_or_else(|e| fail("analyze_new_tracks", e));

    let spec: Option<Value> = args.get(5).map(|spec| {
        serde_json::from_str(&fs::read_to_string(spec).unwrap_or_else(|e| fail("read usb spec", e)))
            .unwrap_or_else(|e| fail("parse usb spec", e))
    });
    // Convert… before the tracks are read, so their rows carry the transcodes.
    let transcodes = spec
        .as_ref()
        .and_then(|spec| spec.get("audioFormats"))
        .map(|formats| convert_tracks(&svc, formats, &ids));

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

    // `get_ui_texts`: the UI's labels, in the language the backend starts in (English).
    let ui_texts =
        serde_json::to_value(ApiResponse::success(backend::i18n::ui_texts())).expect("serialize");
    let mut json = json!({ "tracks": tracks, "detail": detail, "transcodes": transcodes, "uiTexts": ui_texts });
    if let Some(spec) = spec {
        let id_of = |title: &str| {
            tracks
                .iter()
                .find(|t| t.title == title)
                .map(|t| t.id.clone())
                .unwrap_or_else(|| fail("usb spec", format!("no track titled {title:?}")))
        };
        json["usb"] = dump_usb(&svc, &spec, source_dir, id_of);
    }
    fs::write(out, serde_json::to_string(&json).expect("serialize"))
        .unwrap_or_else(|e| fail("write output", e));
}

fn dump_usb(
    svc: &BackendService,
    spec: &Value,
    source_dir: &str,
    id_of: impl Fn(&str) -> String,
) -> Value {
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
        svc.add_library_selection_to_playlist(request(
            "add_library_selection_to_playlist",
            json!({
                "playlistId": created.playlist_id,
                "sourceRoots": [source_dir],
                "trackIds": track_ids
            }),
        ))
        .unwrap_or_else(|e| fail("add_library_selection_to_playlist", e));
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
    let background_color = response(
        "get_usb_background_color",
        svc.get_usb_background_color(request("get_usb_background_color", root.clone())),
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
    let audio_formats = spec
        .get("audioFormats")
        .map(|formats| dump_audio_formats(svc, formats, usb_root, source_dir, &id_of));

    json!({
        "exports": exports,
        "playlists": playlists,
        "playlistTracks": playlist_tracks,
        "histories": histories,
        "playerMenu": player_menu,
        "backgroundColor": background_color,
        "diagnostics": diagnostics,
        "brokenDiagnostics": broken_diagnostics,
        "repairPreview": repair_preview,
        "audioFormats": audio_formats,
    })
}

/// Convert…: the transcode folder `spec.transcodeFolder`, and an MP3 of each
/// track titled in `spec.transcode`. Returns the folder and the dialog's
/// choices, as the Settings drawer and Convert… read them.
fn convert_tracks(svc: &BackendService, spec: &Value, ids: &[String]) -> Value {
    let folder = svc
        .set_transcode_folder(request(
            "set_transcode_folder",
            json!({ "path": spec["transcodeFolder"] }),
        ))
        .unwrap_or_else(|e| fail("set_transcode_folder", e));
    let tracks = svc
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: ids.to_vec(),
        })
        .unwrap_or_else(|e| fail("get_tracks_by_ids_with_previews", e))
        .items;
    let track_ids = spec["transcode"]
        .as_array()
        .expect("transcode")
        .iter()
        .map(|title| {
            let title = title.as_str().expect("title");
            tracks
                .iter()
                .find(|t| t.title == title)
                .map(|t| t.id.clone())
                .unwrap_or_else(|| fail("transcode", format!("no track titled {title:?}")))
        })
        .collect::<Vec<_>>();
    let made = svc
        .transcode_tracks_with_progress(
            request(
                "transcode_tracks",
                json!({ "trackIds": track_ids, "format": "mp3" }),
            ),
            |_| {},
        )
        .unwrap_or_else(|e| fail("transcode_tracks", e));
    if made.transcoded != track_ids.len() {
        fail("transcode_tracks", made.summary);
    }
    json!({
        "folder": response("get_transcode_folder", Ok(folder)),
        "formats": response("get_transcode_formats", Ok(svc.transcode_formats())),
    })
}

/// The USB's Audio formats as set (every one ticked), then with
/// `spec.untick` unticked, and the app playlist `spec.playlist` (not
/// exported) read for that USB: each track's `exportFile`, and the title
/// row's estimate.
fn dump_audio_formats(
    svc: &BackendService,
    spec: &Value,
    usb_root: &str,
    source_dir: &str,
    id_of: impl Fn(&str) -> String,
) -> Value {
    let root = json!({ "usbRoot": usb_root });
    let all_ticked = response(
        "get_usb_audio_formats",
        svc.get_usb_audio_formats(request("get_usb_audio_formats", root.clone())),
    );
    let untick = spec["untick"].as_array().expect("untick");
    let formats = all_ticked["data"]["formats"]
        .as_array()
        .expect("formats")
        .iter()
        .map(|item| {
            json!({
                "format": item["format"],
                "allowed": !untick.contains(&item["format"]),
            })
        })
        .collect::<Vec<_>>();
    let unticked = response(
        "set_usb_audio_formats",
        svc.set_usb_audio_formats(request(
            "set_usb_audio_formats",
            json!({ "usbRoot": usb_root, "formats": formats }),
        )),
    );

    let playlist = &spec["playlist"];
    let created = svc
        .create_playlist(request(
            "create_playlist",
            json!({ "name": playlist["name"] }),
        ))
        .unwrap_or_else(|e| fail("create_playlist", e));
    let track_ids = playlist["titles"]
        .as_array()
        .expect("titles")
        .iter()
        .map(|t| id_of(t.as_str().expect("title")))
        .collect::<Vec<_>>();
    svc.add_library_selection_to_playlist(request(
        "add_library_selection_to_playlist",
        json!({
            "playlistId": created.playlist_id,
            "sourceRoots": [source_dir],
            "trackIds": track_ids
        }),
    ))
    .unwrap_or_else(|e| fail("add_library_selection_to_playlist", e));
    let tracks = response(
        "get_playlist_tracks",
        svc.get_playlist_tracks(request(
            "get_playlist_tracks",
            json!({ "playlistId": created.playlist_id, "usbRoot": usb_root }),
        )),
    );
    let space = response(
        "get_playlist_export_space",
        svc.get_playlist_export_space(request(
            "get_playlist_export_space",
            json!({ "playlistId": created.playlist_id, "usbRoot": usb_root }),
        )),
    );
    // Asked after the playlist exists: the diagnostics dump predates it.
    let export_status = response(
        "refresh_playlist_export_status",
        svc.refresh_playlist_export_status(request("refresh_playlist_export_status", root)),
    );
    json!({
        "allTicked": all_ticked,
        "unticked": unticked,
        "playlistId": created.playlist_id,
        "playlistTracks": tracks,
        "exportSpace": space,
        "exportStatus": export_status,
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
