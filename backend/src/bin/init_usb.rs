use std::env;
use std::path::PathBuf;

use backend::commands::BackendCommands;
use backend::models::InitializeUsbRequest;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!("usage: cargo run --bin init_usb -- <data_dir> <usb_root>");
        std::process::exit(2);
    }

    let data_dir = PathBuf::from(&args[1]);
    let usb_root = PathBuf::from(&args[2]);

    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        eprintln!("failed to create data_dir {}: {err}", data_dir.display());
        std::process::exit(1);
    }
    if let Err(err) = std::fs::create_dir_all(&usb_root) {
        eprintln!("failed to create usb_root {}: {err}", usb_root.display());
        std::process::exit(1);
    }

    let backend = match BackendCommands::new(&data_dir) {
        Ok(v) => v,
        Err(err) => {
            eprintln!(
                "failed to initialize backend at {}: {:?}",
                data_dir.display(),
                err
            );
            std::process::exit(1);
        }
    };

    let response = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    if !response.ok {
        eprintln!("initialize_usb failed: {:?}", response.error);
        std::process::exit(1);
    }

    let data = response.data.expect("initialize_usb response data");
    println!("initialized_usb={}", data.path);
    println!("created_entries={}", data.created_dirs.len());
    for entry in data.created_dirs {
        println!("created={entry}");
    }
}
