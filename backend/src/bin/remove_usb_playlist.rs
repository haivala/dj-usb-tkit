use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::RemoveUsbPlaylistRequest;

fn fail(msg: impl AsRef<str>) -> ! {
    eprintln!("error: {}", msg.as_ref());
    std::process::exit(1);
}

fn default_data_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| fail("$HOME not set"));
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("art.chiph.djusbtkit")
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        fail("usage: remove_usb_playlist <playlist_name> <usb_root> [data_dir]");
    }
    let playlist_name = args[1].clone();
    let usb_root = args[2].clone();
    let data_dir = if args.len() > 3 {
        PathBuf::from(&args[3])
    } else {
        default_data_dir()
    };

    let backend = BackendCommands::new(&data_dir)
        .unwrap_or_else(|e| fail(format!("backend init: {}", e.message)));

    let resp = backend.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: Some(usb_root),
        playlist_id: None,
        playlist_name: playlist_name.clone(),
    });

    if resp.ok {
        let d = resp.data.unwrap();
        println!(
            "removed '{}': tracks_removed={} files_deleted={} kept_shared={}",
            playlist_name, d.tracks_removed, d.files_deleted, d.tracks_kept_shared
        );
        for w in &d.warnings {
            println!("  warning: {}", w.message);
        }
    } else {
        fail(format!(
            "remove failed: {}",
            resp.error.map(|e| e.message).unwrap_or_default()
        ));
    }
}
