use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddSourceFoldersRequest, AddTrackCandidate, AnalyzeNewTracksRequest, AddTrackCandidatesToPlaylistRequest,
    BrowseSourceFilesRequest, CreatePlaylistFolderRequest, CreatePlaylistRequest,
    ExportPlaylistsToUsbRequest, FetchUsbPlaylistsRequest, InitializeUsbRequest,
    MovePlaylistTreeItemRequest, PlaylistTreeItem, PlaylistTreeKind, PlaylistTreePlacement,
};

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn cleanup_temp_data_dir(path: &Path) {
    if !path.exists() {
        return;
    }
    if let Err(err) = std::fs::remove_dir_all(path) {
        eprintln!(
            "warn: failed to remove temp data dir {}: {err}",
            path.display()
        );
    }
}

fn fail_with_cleanup(message: impl AsRef<str>, cleanup_dir: &Path) -> ! {
    cleanup_temp_data_dir(cleanup_dir);
    fail(message);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        fail(
            "usage: cargo run --bin export_usb_to_usb2 -- <usb_source_root> <usb_target_root> (copies all source playlists, in their folders and order)",
        );
    }

    let source_root = canonicalize_or_input(PathBuf::from(&args[1]));
    let target_root = canonicalize_or_input(PathBuf::from(&args[2]));
    if !source_root.is_dir() {
        fail(format!(
            "source root does not exist or is not a directory: {}",
            source_root.display()
        ));
    }
    if !target_root.exists() {
        std::fs::create_dir_all(&target_root).unwrap_or_else(|err| {
            fail(format!(
                "failed to create target directory {}: {err}",
                target_root.display()
            ))
        });
    }

    let project_root = env::current_dir()
        .unwrap_or_else(|err| fail(format!("failed to resolve current directory: {err}")));
    let data_dir = project_root.join(".app-data-export-usb2");
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|err| {
        fail(format!(
            "failed to create data dir {}: {err}",
            data_dir.display()
        ))
    });
    let backend = BackendCommands::new(&data_dir).unwrap_or_else(|err| {
        fail_with_cleanup(
            format!("failed to initialize backend: {}", err.message),
            &data_dir,
        )
    });

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: target_root.to_string_lossy().to_string(),
    });
    if !init.ok {
        fail_with_cleanup(
            format!(
                "failed to initialize target usb: {}",
                init.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ),
            &data_dir,
        );
    }

    let source_contents = source_root.join("Contents");
    let add = backend.add_source_folders(AddSourceFoldersRequest {
        paths: vec![source_contents.to_string_lossy().to_string()],
    });
    // Adding the folder indexes it.
    match add.data.as_ref().filter(|_| add.ok) {
        None => fail_with_cleanup("add_source_folders failed", &data_dir),
        Some(data) if data.scan.is_none() => fail_with_cleanup(
            format!("indexing the source folder failed: {}", data.status_text),
            &data_dir,
        ),
        Some(_) => {}
    }

    // The library view's pages (at most 5000 rows each), until the last one.
    let mut track_map = HashMap::new();
    let mut cursor = None;
    loop {
        let page = backend.browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 5000,
            cursor: cursor.take(),
            ..Default::default()
        });
        let Some(data) = page.data.filter(|_| page.ok) else {
            fail_with_cleanup(
                format!(
                    "browse_source_files failed: {}",
                    page.error
                        .as_ref()
                        .map(|e| e.message.as_str())
                        .unwrap_or("unknown error")
                ),
                &data_dir,
            );
        };
        track_map.extend(
            data.items
                .into_iter()
                .filter_map(|t| canonical_path_key(&t.file_path).map(|key| (key, t.id))),
        );
        if !data.has_more {
            break;
        }
        cursor = data.next_cursor;
    }

    let usb = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(source_root.to_string_lossy().to_string()),
    });
    if !usb.ok {
        fail_with_cleanup(
            format!(
                "fetch_usb_playlists failed: {}",
                usb.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ),
            &data_dir,
        );
    }
    let usb_data = usb
        .data
        .unwrap_or_else(|| fail_with_cleanup("missing fetch_usb_playlists data", &data_dir));
    if usb_data.data.items.is_empty() {
        fail_with_cleanup("no USB playlists found to export", &data_dir);
    }

    // The source stick's tree, rebuilt in the library: its folders, then each
    // playlist in its folder with the tracks matched by path.
    let usb = usb_data.data;
    let items: HashMap<&str, &backend::models::UsbPlaylist> =
        usb.items.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut folders = HashMap::<String, String>::new();
    // Per library folder ("" at the top), its items in the source order.
    let mut siblings = HashMap::<String, Vec<PlaylistTreeItem>>::new();
    let mut playlist_ids = Vec::<String>::new();
    let mut analyze_ids = std::collections::BTreeSet::<String>::new();
    for row in &usb.tree {
        let parent = folders.get(&row.parent_id).cloned();
        if row.kind == PlaylistTreeKind::Folder {
            let created = backend.create_playlist_folder(CreatePlaylistFolderRequest {
                name: row.name.clone(),
                parent_id: parent.clone(),
            });
            let Some(data) = created.data.filter(|_| created.ok) else {
                fail_with_cleanup(format!("creating folder '{}' failed", row.name), &data_dir);
            };
            let folder_id = data.data.folder_id;
            folders.insert(row.id.clone(), folder_id.clone());
            siblings.entry(parent.unwrap_or_default()).or_default().push(PlaylistTreeItem {
                kind: PlaylistTreeKind::Folder,
                id: folder_id,
            });
            continue;
        }
        let Some(usb_playlist) = items.get(row.id.as_str()) else {
            continue;
        };
        let ids: Vec<String> = usb_playlist
            .tracks
            .iter()
            .filter_map(|t| canonical_path_key(&t.file_path))
            .filter_map(|key| track_map.get(&key).cloned())
            .collect();
        analyze_ids.extend(ids.iter().cloned());
        if ids.is_empty() {
            eprintln!(
                "warn: playlist '{}' has no matched local tracks by path",
                usb_playlist.name
            );
            continue;
        }
        let create = backend.create_playlist(CreatePlaylistRequest {
            name: row.name.clone(),
            folder_id: parent.clone(),
        });
        let Some(created) = create.data.filter(|_| create.ok) else {
            eprintln!("warn: skipping playlist '{}': create failed", usb_playlist.name);
            continue;
        };
        let playlist_id = created.data.playlist_id;
        // Candidates keep the USB playlist's order (a library selection would
        // add in library order).
        let add = backend.add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            tracks: ids
                .into_iter()
                .map(|id| AddTrackCandidate {
                    track_id: Some(id.clone()),
                    local_track_id: Some(id),
                    ..Default::default()
                })
                .collect(),
            usb_root: None,
            usb_root_valid: false,
        });
        if !add.ok {
            eprintln!(
                "warn: skipping playlist '{}': adding its tracks failed: {}",
                usb_playlist.name,
                add.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            );
            continue;
        }
        siblings.entry(parent.unwrap_or_default()).or_default().push(PlaylistTreeItem {
            kind: PlaylistTreeKind::Playlist,
            id: playlist_id.clone(),
        });
        playlist_ids.push(playlist_id);
    }
    if playlist_ids.is_empty() {
        fail_with_cleanup("no playlist had tracks to export", &data_dir);
    }

    // Export takes analyzed tracks only: analyze the ones it will write.
    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids: analyze_ids.into_iter().collect(),
        ..Default::default()
    });
    if !analyze.ok {
        fail_with_cleanup("analyzing the tracks failed", &data_dir);
    }

    // New items go first in their folder: each after the one before it puts
    // them back in the source order.
    for items in siblings.values() {
        for pair in items.windows(2) {
            let moved = backend.move_playlist_tree_item(MovePlaylistTreeItemRequest {
                item: pair[1].clone(),
                target: pair[0].clone(),
                placement: PlaylistTreePlacement::After,
            });
            if !moved.ok {
                fail_with_cleanup("ordering the rebuilt playlists failed", &data_dir);
            }
        }
    }

    // One export of everything, in the app's (the source's) order.
    let export = backend.export_playlists_to_usb_with_progress(
        ExportPlaylistsToUsbRequest {
            usb_root: Some(target_root.to_string_lossy().to_string()),
            playlist_ids,
            force_playlist_order: true,
            force_track_order: false,
            options: None,
        },
        |_, _, _| {},
    );
    let Some(data) = export.data.filter(|_| export.ok) else {
        fail_with_cleanup(
            format!(
                "export failed: {}",
                export
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ),
            &data_dir,
        );
    };
    let data = data.data;
    println!("{}", data.status_text);
    for warning in &data.warnings {
        println!("  warning: {}", warning.message);
    }
    println!(
        "done: exported {} playlist(s) from {} to {}",
        data.exported_playlists,
        source_root.display(),
        target_root.display()
    );

    cleanup_temp_data_dir(&data_dir);
}

fn canonicalize_or_input(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn canonical_path_key(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(abs) = std::fs::canonicalize(Path::new(trimmed)) {
        return Some(abs.to_string_lossy().to_string());
    }
    Some(trimmed.replace('\\', "/"))
}
