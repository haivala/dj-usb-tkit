/// dev-tool: dump PDB t16 (columns / player menu) decoded rows.
/// Usage: dump_pdb_t16 <usb_root>
#[cfg(feature = "dev-tools")]
fn main() {
    let usb_root = std::env::args()
        .nth(1)
        .expect("usage: dump_pdb_t16 <usb_root>");
    let usb_path = std::path::Path::new(&usb_root);

    match backend::service::export_helpers::load_pdb_t16_decoded(usb_path) {
        Err(e) => {
            eprintln!("Error reading PDB t16: {e}");
            std::process::exit(1);
        }
        Ok(rows) => {
            println!("PDB t16 rows: {}", rows.len());
            for r in &rows {
                println!("  id={:<3} kind={:<5} name={:?}", r.id, r.kind, r.name);
            }
        }
    }
}

#[cfg(not(feature = "dev-tools"))]
fn main() {
    eprintln!("Requires --features dev-tools");
    std::process::exit(1);
}
