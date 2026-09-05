//! Compares ANLZ beat-grid (first-beat) data for the same tracks across
//! three USB export roots -- typically a Rekordbox-made reference export
//! and two exports from this app (before/after a beat-detection change).
//!
//! Bundle resolution mirrors `render_3way_waveform.rs`: each root's own
//! `exportLibrary.db` is the source of truth for `content.analysisDataFilePath`,
//! since that's exactly what a CDJ follows, rather than re-deriving the
//! hash-bucket path independently.
//!
//! Tracks are matched across roots by `content.fileSize`, not by path.
//! Each export truncates/sanitizes long CDJ filenames independently (see
//! `docs/USB_EXPORT.md`), so the same source track can end up with a
//! different on-USB filename per export -- but since the audio itself is
//! copied byte-for-byte (no transcoding), its file size is stable and a far
//! more reliable join key than the exported name.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use backend::service::anlz::{read_beatgrid_tempo_from_anlz, read_first_beat_from_anlz};
use rusqlite::{Connection, types::ValueRef};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

const WORST_N: usize = 15;

struct RootLabel {
    name: &'static str,
    root: PathBuf,
}

/// Beat-grid readout for one ANLZ bundle (DAT + EXT first-beat/tempo).
#[derive(Default, Clone, Copy)]
struct BundleBeatGrid {
    dat_first_beat_ms: Option<u32>,
    dat_tempo_x100: Option<u16>,
    ext_first_beat_ms: Option<u32>,
    ext_tempo_x100: Option<u16>,
}

impl BundleBeatGrid {
    fn dat_ext_diverge(&self) -> bool {
        matches!(
            (self.dat_first_beat_ms, self.ext_first_beat_ms),
            (Some(d), Some(e)) if d != e
        )
    }
}

/// One matched track: `content.path` + `analysisDataFilePath` as recorded
/// in a single root's own eDB.
#[derive(Clone)]
struct TrackInfo {
    path: String,
    analysis_path: String,
}

struct TrackRow {
    file_size: i64,
    ref_path: String,
    old_path: String,
    new_path: String,
    ref_grid: BundleBeatGrid,
    old_grid: BundleBeatGrid,
    new_grid: BundleBeatGrid,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum Classification {
    Improved,
    Regressed,
    Unchanged,
    Missing,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "usage: cargo run --features dev-tools --bin compare_anlz_beatgrids -- \
             <ref_root> <old_root> <new_root> [--csv out.csv] [--tolerance-ms 10]"
        );
        std::process::exit(2);
    }

    let ref_root = PathBuf::from(&args[1]);
    let old_root = PathBuf::from(&args[2]);
    let new_root = PathBuf::from(&args[3]);

    let mut csv_path: Option<PathBuf> = None;
    let mut tolerance_ms: u32 = 10;
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "--csv" => {
                i += 1;
                csv_path = args.get(i).map(PathBuf::from);
            }
            "--tolerance-ms" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<u32>().ok()) {
                    tolerance_ms = v;
                }
            }
            other => eprintln!("warning: ignoring unknown argument '{other}'"),
        }
        i += 1;
    }

    let labels = [
        RootLabel {
            name: "REF",
            root: ref_root,
        },
        RootLabel {
            name: "OLD",
            root: old_root,
        },
        RootLabel {
            name: "NEW",
            root: new_root,
        },
    ];

    let maps: Vec<BTreeMap<i64, TrackInfo>> = labels
        .iter()
        .map(|l| match load_track_map(&l.root) {
            Ok(m) => {
                println!("{}: {} tracks in eDB ({})", l.name, m.len(), l.root.display());
                m
            }
            Err(e) => {
                eprintln!("error: cannot load eDB from {}: {e}", l.root.display());
                std::process::exit(1);
            }
        })
        .collect();

    let matched_keys: Vec<i64> = maps[0]
        .keys()
        .filter(|k| maps[1].contains_key(*k) && maps[2].contains_key(*k))
        .copied()
        .collect();

    report_unmatched(&labels, &maps, &matched_keys);
    println!("\nmatched across all three roots (by file size): {}\n", matched_keys.len());

    let mut rows = Vec::with_capacity(matched_keys.len());
    for key in &matched_keys {
        let ref_info = &maps[0][key];
        let old_info = &maps[1][key];
        let new_info = &maps[2][key];
        let ref_grid = read_bundle(&labels[0].root, &ref_info.analysis_path);
        let old_grid = read_bundle(&labels[1].root, &old_info.analysis_path);
        let new_grid = read_bundle(&labels[2].root, &new_info.analysis_path);
        rows.push(TrackRow {
            file_size: *key,
            ref_path: ref_info.path.clone(),
            old_path: old_info.path.clone(),
            new_path: new_info.path.clone(),
            ref_grid,
            old_grid,
            new_grid,
        });
    }

    print_dat_ext_divergences(&labels, &rows);
    print_summary(&rows, tolerance_ms);
    print_worst(&rows, tolerance_ms);

    if let Some(path) = csv_path {
        if let Err(e) = write_csv(&path, &rows, tolerance_ms) {
            eprintln!("error: failed writing CSV to {}: {e}", path.display());
            std::process::exit(1);
        }
        println!("\nwrote per-track CSV to {}", path.display());
    }
}

fn report_unmatched(
    labels: &[RootLabel; 3],
    maps: &[BTreeMap<i64, TrackInfo>],
    matched: &[i64],
) {
    for (i, label) in labels.iter().enumerate() {
        let unmatched: Vec<&TrackInfo> = maps[i]
            .iter()
            .filter(|(k, _)| !matched.contains(k))
            .map(|(_, v)| v)
            .collect();
        if !unmatched.is_empty() {
            println!(
                "\n{} has {} track(s) not present (by file size) in all three roots:",
                label.name,
                unmatched.len()
            );
            for info in unmatched.iter().take(20) {
                println!("  {}", info.path);
            }
            if unmatched.len() > 20 {
                println!("  ... and {} more", unmatched.len() - 20);
            }
        }
    }
}

/// Distance between two beat-grid values, wrapped circularly around the
/// track's own beat interval when known. Plain absolute difference is
/// wrong here: `anlz::normalize_first_beat_ms` always wraps `first_beat_ms`
/// into `[0, interval)`, so a value just past the wrap boundary (e.g.
/// interval=487.8ms, phase 507ms -> stored as 19ms) is only a few ms away
/// from a phase near the top of the interval (e.g. 459ms) in real musical
/// time, even though the raw stored numbers look ~440ms apart.
fn delta_ms(a: Option<u32>, b: Option<u32>, interval_ms: Option<f64>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => match interval_ms.filter(|i| i.is_finite() && *i > 1.0) {
            Some(interval_ms) => {
                let raw = (a as f64 - b as f64).abs() % interval_ms;
                Some(raw.min(interval_ms - raw).round() as u32)
            }
            None => Some(a.abs_diff(b)),
        },
        _ => None,
    }
}

fn classify(row: &TrackRow, tolerance_ms: u32) -> (Option<u32>, Option<u32>, Classification) {
    let interval_ms = row
        .ref_grid
        .dat_tempo_x100
        .filter(|&t| t > 0)
        .map(|t| 6_000_000.0 / t as f64);
    let delta_old = delta_ms(row.old_grid.dat_first_beat_ms, row.ref_grid.dat_first_beat_ms, interval_ms);
    let delta_new = delta_ms(row.new_grid.dat_first_beat_ms, row.ref_grid.dat_first_beat_ms, interval_ms);
    let class = match (delta_old, delta_new) {
        (Some(o), Some(n)) => {
            if n.abs_diff(o) <= tolerance_ms {
                Classification::Unchanged
            } else if n < o {
                Classification::Improved
            } else {
                Classification::Regressed
            }
        }
        _ => Classification::Missing,
    };
    (delta_old, delta_new, class)
}

fn print_dat_ext_divergences(labels: &[RootLabel; 3], rows: &[TrackRow]) {
    let grids = |row: &TrackRow| [row.ref_grid, row.old_grid, row.new_grid];
    fn display_path(row: &TrackRow, i: usize) -> &str {
        match i {
            0 => &row.ref_path,
            1 => &row.old_path,
            _ => &row.new_path,
        }
    }
    for (i, label) in labels.iter().enumerate() {
        let diverging: Vec<&TrackRow> = rows
            .iter()
            .filter(|r| grids(r)[i].dat_ext_diverge())
            .collect();
        if !diverging.is_empty() {
            println!(
                "\n{} DAT/EXT first-beat disagreement in {} track(s):",
                label.name,
                diverging.len()
            );
            for row in diverging.iter().take(10) {
                let g = grids(row)[i];
                println!(
                    "  {} dat={:?}ms ext={:?}ms",
                    display_path(row, i),
                    g.dat_first_beat_ms,
                    g.ext_first_beat_ms
                );
            }
        }
    }
}

fn print_summary(rows: &[TrackRow], tolerance_ms: u32) {
    let mut deltas_old = Vec::new();
    let mut deltas_new = Vec::new();
    let mut improved = 0usize;
    let mut regressed = 0usize;
    let mut unchanged = 0usize;
    let mut missing = 0usize;

    for row in rows {
        let (delta_old, delta_new, class) = classify(row, tolerance_ms);
        if let Some(d) = delta_old {
            deltas_old.push(d);
        }
        if let Some(d) = delta_new {
            deltas_new.push(d);
        }
        match class {
            Classification::Improved => improved += 1,
            Classification::Regressed => regressed += 1,
            Classification::Unchanged => unchanged += 1,
            Classification::Missing => missing += 1,
        }
    }

    println!("=== Summary (first-beat delta vs REF, tolerance {tolerance_ms}ms) ===");
    print_stats("OLD", &deltas_old);
    print_stats("NEW", &deltas_new);
    println!(
        "improved={improved} regressed={regressed} unchanged={unchanged} missing={missing} \
         (total {})",
        rows.len()
    );
}

fn print_stats(label: &str, deltas: &[u32]) {
    if deltas.is_empty() {
        println!("{label}: no data");
        return;
    }
    let mut sorted = deltas.to_vec();
    sorted.sort_unstable();
    let sum: u64 = sorted.iter().map(|&d| d as u64).sum();
    let mean = sum as f64 / sorted.len() as f64;
    let median = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap();
    println!(
        "{label}: mean={mean:.1}ms median={median}ms max={max}ms (n={})",
        sorted.len()
    );
}

struct RankedRow<'a> {
    row: &'a TrackRow,
    delta_old: Option<u32>,
    delta_new: Option<u32>,
    class: Classification,
    magnitude: i64,
}

fn print_worst(rows: &[TrackRow], tolerance_ms: u32) {
    let mut ranked: Vec<RankedRow<'_>> = rows
        .iter()
        .map(|row| {
            let (delta_old, delta_new, class) = classify(row, tolerance_ms);
            let magnitude = match (delta_old, delta_new) {
                (Some(o), Some(n)) => n as i64 - o as i64,
                _ => 0,
            };
            RankedRow {
                row,
                delta_old,
                delta_new,
                class,
                magnitude,
            }
        })
        .collect();

    ranked.sort_by_key(|r| -r.magnitude);
    println!("\n=== Worst regressions (NEW moved away from REF) ===");
    for r in ranked.iter().take(WORST_N) {
        if r.class != Classification::Regressed {
            continue;
        }
        print_row_detail(r.row, r.delta_old, r.delta_new, r.magnitude);
    }

    ranked.sort_by_key(|r| r.magnitude);
    println!("\n=== Best improvements (NEW moved toward REF) ===");
    for r in ranked.iter().take(WORST_N) {
        if r.class != Classification::Improved {
            continue;
        }
        print_row_detail(r.row, r.delta_old, r.delta_new, r.magnitude);
    }
}

fn print_row_detail(row: &TrackRow, delta_old: Option<u32>, delta_new: Option<u32>, mag: i64) {
    println!(
        "  {} (ref path: {}) ref={:?}ms old={:?}ms new={:?}ms delta_old={delta_old:?} \
         delta_new={delta_new:?} change={mag:+}ms",
        row.file_size,
        row.ref_path,
        row.ref_grid.dat_first_beat_ms,
        row.old_grid.dat_first_beat_ms,
        row.new_grid.dat_first_beat_ms
    );
}

fn write_csv(path: &Path, rows: &[TrackRow], tolerance_ms: u32) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(
        "file_size,ref_path,old_path,new_path,ref_dat_ms,old_dat_ms,new_dat_ms,\
         ref_ext_ms,old_ext_ms,new_ext_ms,delta_old_ms,delta_new_ms,classification\n",
    );
    for row in rows {
        let (delta_old, delta_new, class) = classify(row, tolerance_ms);
        let class_str = match class {
            Classification::Improved => "improved",
            Classification::Regressed => "regressed",
            Classification::Unchanged => "unchanged",
            Classification::Missing => "missing",
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            row.file_size,
            csv_escape(&row.ref_path),
            csv_escape(&row.old_path),
            csv_escape(&row.new_path),
            opt_str(row.ref_grid.dat_first_beat_ms),
            opt_str(row.old_grid.dat_first_beat_ms),
            opt_str(row.new_grid.dat_first_beat_ms),
            opt_str(row.ref_grid.ext_first_beat_ms),
            opt_str(row.old_grid.ext_first_beat_ms),
            opt_str(row.new_grid.ext_first_beat_ms),
            opt_str(delta_old),
            opt_str(delta_new),
            class_str,
        ));
    }
    fs::write(path, out)
}

fn opt_str<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn read_bundle(usb_root: &Path, analysis_path: &str) -> BundleBeatGrid {
    let dat_path = resolve_ext(usb_root, analysis_path, "DAT");
    let ext_path = resolve_ext(usb_root, analysis_path, "EXT");

    let mut grid = BundleBeatGrid::default();
    if let Ok(bytes) = fs::read(&dat_path) {
        grid.dat_first_beat_ms = read_first_beat_from_anlz(&bytes);
        grid.dat_tempo_x100 = read_beatgrid_tempo_from_anlz(&bytes);
    }
    if let Ok(bytes) = fs::read(&ext_path) {
        grid.ext_first_beat_ms = read_first_beat_from_anlz(&bytes);
        grid.ext_tempo_x100 = read_beatgrid_tempo_from_anlz(&bytes);
    }
    grid
}

// ---------------------------------------------------------------------------
// eDB helpers (mirrors render_3way_waveform.rs)
// ---------------------------------------------------------------------------

/// Maps `content.fileSize -> {path, analysisDataFilePath}` for one USB
/// root's eDB. Keyed by file size rather than path -- see module doc for
/// why path isn't a reliable cross-export join key. A handful of
/// intra-root file-size collisions are expected (e.g. the same physical
/// file deliberately present at two library locations); the first row
/// encountered wins, which is harmless since such collisions share
/// identical audio and therefore identical beat-grid data.
fn load_track_map(usb_root: &Path) -> Result<BTreeMap<i64, TrackInfo>, String> {
    let db = usb_root
        .join("PIONEER")
        .join("rekordbox")
        .join("exportLibrary.db");
    let conn = open_with_known_keys(&db)?;
    let mut stmt = conn
        .prepare(
            "SELECT path, analysisDataFilePath, fileSize FROM content \
             WHERE path IS NOT NULL AND TRIM(path) != '' AND fileSize IS NOT NULL",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("query: {e}"))?;
    let mut out = BTreeMap::new();
    while let Ok(Some(row)) = rows.next() {
        let path = render_value_ref(row.get_ref(0).map_err(|e| e.to_string())?);
        let anlz = render_value_ref(row.get_ref(1).map_err(|e| e.to_string())?);
        let file_size: i64 = row.get(2).map_err(|e| e.to_string())?;
        if path.is_empty() || anlz.is_empty() || file_size == 0 {
            continue;
        }
        out.entry(file_size).or_insert(TrackInfo {
            path,
            analysis_path: anlz,
        });
    }
    Ok(out)
}

fn open_with_known_keys(path: &Path) -> Result<Connection, String> {
    let open_plain = || Connection::open(path).map_err(|e| e.to_string());
    let has_schema = |conn: &Connection| {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
            > 0
    };
    let plain = open_plain()?;
    if has_schema(&plain) {
        return Ok(plain);
    }
    for key in [DEFAULT_MASTER_KEY, DEFAULT_USB_EXPORT_KEY] {
        let conn = open_plain()?;
        if conn.execute_batch(&format!("PRAGMA key='{key}';")).is_err() {
            continue;
        }
        if has_schema(&conn) {
            return Ok(conn);
        }
    }
    Err(format!("cannot open database {}", path.display()))
}

fn render_value_ref(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(x) => x.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(x) => String::from_utf8_lossy(x).to_string(),
        ValueRef::Blob(x) => format!("<blob:{}>", x.len()),
    }
}

fn resolve_ext(usb_root: &Path, analysis_path: &str, ext: &str) -> PathBuf {
    let rel = analysis_path.trim_start_matches('/').replace('\\', "/");
    let base = usb_root.join(rel);
    base.with_extension(ext)
}
