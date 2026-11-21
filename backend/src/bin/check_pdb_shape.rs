//! Lists what in a PDB rekordbox or the players can't read: the check every
//! write passes before it reaches the USB (`pdb_write_problems`). With a
//! second PDB, only what the first has that the second doesn't.
//!
//! Usage:
//!   cargo run --features dev-tools --bin check_pdb_shape -- <export.pdb> [<before.pdb>]
use std::env;
use std::process::exit;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("usage: check_pdb_shape <export.pdb> [<before.pdb>]");
        exit(2);
    }
    let read = |path: &str| {
        std::fs::read(path).unwrap_or_else(|err| {
            eprintln!("can't read {path}: {err}");
            exit(2);
        })
    };
    let after = read(&args[1]);
    let problems = match args.get(2) {
        Some(before) => backend::service::pdb_write_problems(Some(&read(before)), &after),
        None => backend::service::pdb_shape_problems(&after),
    };
    for problem in &problems {
        println!("{problem}");
    }
    println!("{} problems", problems.len());
    exit(i32::from(!problems.is_empty()));
}
