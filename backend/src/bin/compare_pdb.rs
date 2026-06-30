/// compare_pdb <pdb-a> <pdb-b>
///
/// Byte-level diff between two PDB files annotated with page/offset context.
/// Reports changed bytes grouped by page.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: compare_pdb <pdb-a> <pdb-b>");
        std::process::exit(1);
    }
    let a = std::fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[1]);
        std::process::exit(1);
    });
    let b = std::fs::read(&args[2]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[2]);
        std::process::exit(1);
    });

    let page_size: usize = if a.len() >= 8 {
        let v = u32::from_le_bytes(a[4..8].try_into().unwrap()) as usize;
        if v == 4096 { v } else { 4096 }
    } else {
        4096
    };

    let total = a.len().max(b.len());
    let mut diffs: Vec<(usize, u8, u8)> = Vec::new();
    for i in 0..total {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        if av != bv {
            diffs.push((i, av, bv));
        }
    }

    println!("a={}", args[1]);
    println!("b={}", args[2]);
    println!("total_diffs={}", diffs.len());

    if diffs.is_empty() {
        println!("identical");
        return;
    }

    // Group by page
    let mut cur_page = usize::MAX;
    for (abs, av, bv) in &diffs {
        let page = abs / page_size;
        let off = abs % page_size;
        if page != cur_page {
            cur_page = page;
            println!("\npage[{page}]:");
        }
        println!("  +0x{off:04x} ({abs:#08x})  a=0x{av:02x}  b=0x{bv:02x}");
    }
}
