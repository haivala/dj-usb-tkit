use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, DedupeMode, ExportToUsbRequest, FetchUsbPlaylistsRequest,
    InitializeUsbRequest, ScanLibraryRequest, SearchTracksRequest,
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
            "usage: cargo run --bin export_usb_to_usb2 -- <usb_source_root> <usb_target_root> (copies all source playlists)",
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
    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_contents.to_string_lossy().to_string()],
        incremental: true,
    });
    if !scan.ok {
        fail_with_cleanup(
            format!(
                "scan_library failed: {}",
                scan.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ),
            &data_dir,
        );
    }

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 200_000,
        cursor: None,
    });
    if !search.ok {
        fail_with_cleanup(
            format!(
                "search_tracks failed: {}",
                search
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            ),
            &data_dir,
        );
    }
    let track_map = search
        .data
        .map(|d| {
            d.items
                .into_iter()
                .filter_map(|t| canonical_path_key(&t.file_path).map(|key| (key, t.id)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

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
    if usb_data.items.is_empty() {
        fail_with_cleanup("no USB playlists found to export", &data_dir);
    }

    let mut exported_count = 0usize;
    for usb_playlist in usb_data.items {
        let create = backend.create_playlist(backend::models::CreatePlaylistRequest {
            name: usb_playlist.name.clone(),
        });
        if !create.ok {
            eprintln!(
                "warn: skipping playlist '{}': create failed: {}",
                usb_playlist.name,
                create
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            );
            continue;
        }
        let playlist_id = create
            .data
            .as_ref()
            .map(|d| d.playlist_id.clone())
            .unwrap_or_else(|| fail_with_cleanup("missing create_playlist data", &data_dir));

        let mut ids = Vec::<String>::new();
        for t in &usb_playlist.tracks {
            let Some(key) = canonical_path_key(&t.file_path) else {
                continue;
            };
            if let Some(id) = track_map.get(&key) {
                ids.push(id.clone());
            }
        }
        if ids.is_empty() {
            eprintln!(
                "warn: playlist '{}' has no matched local tracks by path",
                usb_playlist.name
            );
            continue;
        }

        let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            track_ids: ids,
            dedupe: DedupeMode::Skip,
        });
        if !add.ok {
            eprintln!(
                "warn: skipping playlist '{}': add_tracks_to_playlist failed: {}",
                usb_playlist.name,
                add.error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            );
            continue;
        }

        let export = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(target_root.to_string_lossy().to_string()),
            playlist_id: playlist_id.clone(),
            options: None,
        });
        if !export.ok {
            eprintln!(
                "warn: export failed for '{}': {}",
                usb_playlist.name,
                export
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unknown error")
            );
            continue;
        }
        let data = export
            .data
            .unwrap_or_else(|| fail_with_cleanup("missing export_to_usb data", &data_dir));
        exported_count += 1;
        println!(
            "exported '{}' -> {} (tracks={}, skipped={})",
            data.playlist_name, data.usb_root, data.exported_tracks, data.skipped_tracks
        );
        if !data.warnings.is_empty() {
            let lines = data
                .warnings
                .iter()
                .map(|w| w.message.as_str())
                .collect::<Vec<_>>();
            println!("  warnings: {}", lines.join(" | "));
        }
    }

    println!(
        "done: exported {} playlist(s) from {} to {}",
        exported_count,
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
