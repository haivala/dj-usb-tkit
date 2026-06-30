use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::{ExportToUsbRequest, InitializeUsbRequest};

const APP_IDENTIFIER: &str = "art.chiph.djusbtkit";

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn default_data_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| fail("$HOME is not set"));
    #[cfg(target_os = "macos")]
    return PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APP_IDENTIFIER);
    #[cfg(not(target_os = "macos"))]
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join(APP_IDENTIFIER)
}

fn resolve_data_dir(arg: Option<PathBuf>) -> PathBuf {
    let Some(path) = arg else {
        return default_data_dir();
    };

    if path
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("db"))
        .unwrap_or(false)
    {
        return path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| fail("database path must have a parent directory"));
    }

    path
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 || args.len() > 5 {
        fail(
            "usage: cargo run --bin export_playlist_to_usb -- <playlist_name> <usb_root> [data_dir|backend.db] [--dry-run]",
        );
    }

    let playlist_name = args[1].trim().to_string();
    let usb_root = PathBuf::from(&args[2]);

    let mut data_dir_arg: Option<PathBuf> = None;
    let mut dry_run = false;
    for arg in &args[3..] {
        if arg == "--dry-run" {
            dry_run = true;
        } else if data_dir_arg.is_none() {
            data_dir_arg = Some(PathBuf::from(arg));
        } else {
            fail(format!("unexpected argument: {arg}"));
        }
    }

    let data_dir = resolve_data_dir(data_dir_arg);

    if playlist_name.is_empty() {
        fail("playlist_name must not be empty");
    }

    std::fs::create_dir_all(&data_dir).unwrap_or_else(|err| {
        fail(format!(
            "failed to create data dir {}: {err}",
            data_dir.display()
        ))
    });
    std::fs::create_dir_all(&usb_root).unwrap_or_else(|err| {
        fail(format!(
            "failed to create usb root {}: {err}",
            usb_root.display()
        ))
    });

    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|err| fail(format!("failed to initialize backend: {}", err.message)));

    let playlists = backend.list_playlists();
    if !playlists.ok {
        fail(format!(
            "failed to list playlists: {}",
            playlists
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }
    let items = playlists
        .data
        .unwrap_or_else(|| fail("missing list_playlists data"))
        .items;
    let exact_matches = items
        .iter()
        .filter(|p| p.name == playlist_name)
        .collect::<Vec<_>>();
    let insensitive_matches = if exact_matches.is_empty() {
        let needle = playlist_name.to_ascii_lowercase();
        items
            .iter()
            .filter(|p| p.name.to_ascii_lowercase() == needle)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let playlist = if exact_matches.len() == 1 {
        exact_matches[0]
    } else if exact_matches.len() > 1 {
        let ids = exact_matches
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        fail(format!(
            "playlist name {:?} is ambiguous (multiple exact matches): {}",
            playlist_name, ids
        ));
    } else if insensitive_matches.len() == 1 {
        insensitive_matches[0]
    } else if insensitive_matches.len() > 1 {
        let labels = insensitive_matches
            .iter()
            .map(|p| format!("{} ({})", p.name, p.id))
            .collect::<Vec<_>>()
            .join(", ");
        fail(format!(
            "playlist name {:?} is ambiguous (case-insensitive matches): {}",
            playlist_name, labels
        ));
    } else {
        let names = items.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        fail(format!(
            "no playlist named {:?}; available: {}",
            playlist_name,
            if names.is_empty() {
                "(none)".to_string()
            } else {
                names.join(", ")
            }
        ));
    };
    let playlist_id = playlist.id.clone();

    if dry_run {
        // Use process-local env toggle consumed by export service.
        unsafe {
            env::set_var("USB_EXPORT_DRY_RUN", "1");
        }
    }

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    if !init.ok {
        fail(format!(
            "failed to initialize usb: {}",
            init.error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb_root.to_string_lossy().to_string()),
        playlist_id,
        options: None,
    });
    if !export.ok {
        fail(format!(
            "export_to_usb failed: {}",
            export
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error")
        ));
    }

    let data = export
        .data
        .unwrap_or_else(|| fail("missing export_to_usb data"));
    if dry_run {
        println!(
            "dry-run '{}' -> {} (tracks={}, skipped={})",
            data.playlist_name, data.usb_root, data.exported_tracks, data.skipped_tracks
        );
    } else {
        println!(
            "exported '{}' -> {} (tracks={}, skipped={})",
            data.playlist_name, data.usb_root, data.exported_tracks, data.skipped_tracks
        );
    }
    if !data.warnings.is_empty() {
        let warnings = data
            .warnings
            .iter()
            .map(|w| w.message.as_str())
            .collect::<Vec<_>>();
        println!("warnings: {}", warnings.join(" | "));
    }
    println!("manifest: {}", data.manifest_path);
}
