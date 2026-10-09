//! Write the Engine DJ library (`Engine Library/Database2`) for a USB from
//! its rekordbox export (`PIONEER/rekordbox/export.pdb` + ANLZ), for Engine OS
//! players (Denon Prime, Numark Mixstream).

use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: write_engine_db <usb_root>");
        std::process::exit(2);
    }
    match backend::service::rebuild_engine_library(&PathBuf::from(&args[1])) {
        // Warnings are already logged to stderr.
        Ok(data) => {
            println!(
                "wrote {} tracks ({} analyzed), {} playlists",
                data.tracks, data.analyzed_tracks, data.playlists
            );
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
