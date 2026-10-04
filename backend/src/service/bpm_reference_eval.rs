//! BPM accuracy against a rekordbox-analyzed USB (opt-in, `#[ignore]`d).
//!
//! Every track of one playlist is analyzed the way the app does and compared
//! with the BPM rekordbox wrote to `export.pdb`:
//!
//! ```text
//! BPM_REFERENCE_PDB=/path/to/USB/PIONEER/rekordbox/export.pdb \
//! BPM_REFERENCE_PLAYLIST="<playlist name>" \
//!   cargo test --release -p backend bpm_reference -- --ignored --nocapture
//! ```
//!
//! (`--release`: unoptimized, stratum takes minutes per track.)
//!
//! Or, for BPMs from elsewhere (such as rekordbox's desktop library),
//! `BPM_REFERENCE_LIST=<file>` with one `path<TAB>bpm<TAB>title` per line.
//!
//! Optional: `BPM_REFERENCE_RANGE` (default `70-180`), `BPM_REFERENCE_JOBS`
//! (default 6), `BPM_REFERENCE_SAMPLE` (analyze only this many tracks, evenly
//! spread), `BPM_REFERENCE_OUT` (per-track CSV) and `BPM_REFERENCE_CACHE`
//! (stratum's raw estimates, reused by later runs so tuning `refine_bpm`
//! skips stratum).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::analysis::{ANALYSIS_DECODE_MAX_SAMPLES, decode_audio_mono_samples};
use super::anlz::{read_beatgrid_tempo_from_anlz, read_first_beat_from_anlz};
use super::bpm_key::{
    find_tempo_peak, first_beat_secs, refine_bpm, refine_near_estimate, stratum_config,
    tempo_family_alternative,
};
use crate::pdb_reader::parse_pdb;

struct Reference {
    title: String,
    path: PathBuf,
    bpm: f64,
    /// rekordbox's beat grid: first beat (ms) and grid BPM.
    grid: Option<(f64, f64)>,
}

/// stratum's raw BPM and first beat (seconds) for one track.
type StratumResult = (f32, Option<f64>);

struct Outcome {
    title: String,
    path: PathBuf,
    reference: f64,
    raw: Option<f32>,
    refined: Option<f64>,
    /// Fine-search peak and grid coherence relative to it, for tuning.
    peak: Option<(f64, f64)>,
    /// Closest 3:2 / 4:3 alternative: its margin (1.0 = switches), and the
    /// BPM refined around it, for tuning `TEMPO_FAMILY`.
    family: Option<(f64, f64)>,
    grid: Option<(f64, f64)>,
    /// stratum's first beat and ours (seconds).
    first_beat: Option<f64>,
    own_first_beat: Option<f64>,
}

/// Distance to the reference in BPM, ignoring the octave: the analysis
/// keeps stratum's octave, and rekordbox's is no more right than ours.
fn octave_error(bpm: Option<f64>, reference: f64) -> Option<f64> {
    let bpm = bpm?;
    [bpm, bpm * 2.0, bpm / 2.0]
        .iter()
        .map(|v| (v - reference).abs())
        .reduce(f64::min)
}

/// rekordbox's BPM is a whole or half value rather than one between.
fn on_grid(reference: f64) -> bool {
    (reference * 2.0 - (reference * 2.0).round()).abs() < 0.005
}

fn load_references(pdb: &Path, playlist: &str) -> Vec<Reference> {
    let usb_root = pdb
        .ancestors()
        .nth(3)
        .expect("export.pdb lives in <usb>/PIONEER/rekordbox/");
    let parsed = parse_pdb(pdb).expect("parse export.pdb");
    let playlist_ids = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder && row.name == playlist)
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert!(
        !playlist_ids.is_empty(),
        "playlist '{playlist}' not found; similar: {:?}",
        parsed
            .playlist_tree
            .iter()
            .filter(|row| {
                let name = row.name.to_lowercase();
                playlist
                    .to_lowercase()
                    .split_whitespace()
                    .any(|word| name.contains(word))
            })
            .map(|row| &row.name)
            .collect::<Vec<_>>()
    );
    let tracks = parsed
        .tracks
        .iter()
        .map(|t| (t.id, t))
        .collect::<HashMap<_, _>>();
    let mut entries = parsed
        .playlist_entries
        .iter()
        .filter(|e| playlist_ids.contains(&e.playlist_id))
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.entry_index);
    entries
        .iter()
        .filter_map(|e| tracks.get(&e.track_id))
        .filter(|t| t.tempo_x100 > 0)
        .map(|t| Reference {
            title: t.title.clone(),
            path: usb_root.join(t.track_file_path.trim_start_matches('/')),
            bpm: f64::from(t.tempo_x100) / 100.0,
            grid: std::fs::read(usb_root.join(t.anlz_path.trim_start_matches('/')))
                .ok()
                .and_then(|anlz| {
                    let first = read_first_beat_from_anlz(&anlz)?;
                    let tempo = read_beatgrid_tempo_from_anlz(&anlz)?;
                    Some((f64::from(first), f64::from(tempo) / 100.0))
                }),
        })
        .collect()
}

fn load_reference_list(path: &Path) -> Vec<Reference> {
    std::fs::read_to_string(path)
        .expect("read BPM_REFERENCE_LIST")
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let path = PathBuf::from(parts.next()?);
            let bpm = parts.next()?.trim().parse().ok()?;
            let title = parts.next().unwrap_or_default().to_string();
            Some(Reference {
                title,
                path,
                bpm,
                grid: None,
            })
        })
        .collect()
}

fn load_cache(path: Option<&Path>) -> HashMap<(String, String), StratumResult> {
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let range = parts.next()?.to_string();
            let path = parts.next()?.to_string();
            let raw = parts.next()?.parse().ok()?;
            let first_beat = parts.next().and_then(|v| v.parse().ok());
            Some(((range, path), (raw, first_beat)))
        })
        .collect()
}

fn analyze(
    reference: &Reference,
    cached: Option<StratumResult>,
    bpm_min: u32,
    bpm_max: u32,
) -> Outcome {
    let outcome = |stratum: Option<StratumResult>, refined, peak, family, own_first_beat| Outcome {
        title: reference.title.clone(),
        path: reference.path.clone(),
        reference: reference.bpm,
        raw: stratum.map(|s| s.0),
        refined,
        peak,
        family,
        grid: reference.grid,
        first_beat: stratum.and_then(|s| s.1),
        own_first_beat,
    };
    let Ok((samples, sample_rate)) =
        decode_audio_mono_samples(&reference.path, ANALYSIS_DECODE_MAX_SAMPLES)
    else {
        return outcome(None, None, None, None, None);
    };
    // Older cache lines have no first beat: run stratum again for those.
    let stratum = cached.filter(|c| c.1.is_some()).or_else(|| {
        stratum_dsp::analyze_audio(&samples, sample_rate, stratum_config(bpm_min, bpm_max))
            .ok()
            .filter(|r| r.bpm > 0.0)
            .map(|r| (r.bpm, r.beat_grid.beats.first().map(|&t| f64::from(t))))
    });
    let raw = stratum.map(|s| s.0);
    let refined = raw.map(|raw| refine_bpm(&samples, sample_rate, raw, bpm_min, bpm_max));
    let peak = raw
        .and_then(|raw| find_tempo_peak(&samples, sample_rate, raw, bpm_min, bpm_max))
        .map(|p| (p.bpm, p.grid_coherence / p.coherence));
    let family = raw
        .and_then(|raw| tempo_family_alternative(&samples, sample_rate, raw, bpm_min, bpm_max))
        .map(|(alternative, margin)| {
            let refined =
                refine_near_estimate(&samples, sample_rate, alternative, bpm_min, bpm_max);
            (margin, refined)
        });
    let own_first_beat = refined.and_then(|bpm| first_beat_secs(&samples, sample_rate, bpm));
    outcome(stratum, refined, peak, family, own_first_beat)
}

#[test]
#[ignore = "needs reference BPMs; set BPM_REFERENCE_PDB or BPM_REFERENCE_LIST"]
fn bpm_reference_accuracy() {
    let pdb = std::env::var_os("BPM_REFERENCE_PDB").map(PathBuf::from);
    let list = std::env::var_os("BPM_REFERENCE_LIST").map(PathBuf::from);
    if pdb.is_none() && list.is_none() {
        eprintln!("BPM_REFERENCE_PDB / BPM_REFERENCE_LIST not set; skipping");
        return;
    }
    let playlist = std::env::var("BPM_REFERENCE_PLAYLIST").unwrap_or_default();
    let range = std::env::var("BPM_REFERENCE_RANGE").unwrap_or_else(|_| "70-180".into());
    let (bpm_min, bpm_max) = range
        .split_once('-')
        .and_then(|(lo, hi)| Some((lo.trim().parse().ok()?, hi.trim().parse().ok()?)))
        .expect("BPM_REFERENCE_RANGE like 70-180");
    let jobs = std::env::var("BPM_REFERENCE_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6usize);
    let cache_path = std::env::var_os("BPM_REFERENCE_CACHE").map(PathBuf::from);
    let cache = load_cache(cache_path.as_deref());

    let mut references = match (&list, &pdb) {
        (Some(list), _) => load_reference_list(list),
        (None, Some(pdb)) => {
            assert!(!playlist.is_empty(), "set BPM_REFERENCE_PLAYLIST");
            load_references(pdb, &playlist)
        }
        (None, None) => unreachable!(),
    };
    let mut histogram = std::collections::BTreeMap::<u32, usize>::new();
    for r in &references {
        *histogram.entry((r.bpm / 10.0) as u32 * 10).or_default() += 1;
    }
    println!("reference BPMs of all {} tracks:", references.len());
    for (bucket, count) in &histogram {
        println!("  {bucket:>3}-{:<3} {count}", bucket + 9);
    }
    let sample = std::env::var("BPM_REFERENCE_SAMPLE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0 && n < references.len());
    if let Some(n) = sample {
        let stride = references.len() as f64 / n as f64;
        let picked = (0..n)
            .map(|i| (i as f64 * stride) as usize)
            .collect::<std::collections::HashSet<_>>();
        references = references
            .into_iter()
            .enumerate()
            .filter(|(i, _)| picked.contains(i))
            .map(|(_, r)| r)
            .collect();
    }
    eprintln!("{} tracks, range {range}, {jobs} jobs", references.len());

    let next = AtomicUsize::new(0);
    let outcomes = Mutex::new(Vec::with_capacity(references.len()));
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(reference) = references.get(i) else {
                        break;
                    };
                    let key = (range.clone(), reference.path.display().to_string());
                    let outcome = analyze(reference, cache.get(&key).copied(), bpm_min, bpm_max);
                    eprintln!(
                        "[{}/{}] ref {:.2} raw {:?} final {:?}  {}",
                        i + 1,
                        references.len(),
                        outcome.reference,
                        outcome.raw,
                        outcome.refined,
                        outcome.title
                    );
                    outcomes.lock().unwrap().push(outcome);
                }
            });
        }
    });
    let mut outcomes = outcomes.into_inner().unwrap();
    outcomes.sort_by(|a, b| a.path.cmp(&b.path));

    if let Some(cache_path) = &cache_path {
        let mut text = cache
            .iter()
            .filter(|((r, _), _)| *r != range)
            .map(|((r, p), (raw, first))| {
                let first = first.map(|v| v.to_string()).unwrap_or_default();
                format!("{r}\t{p}\t{raw}\t{first}\n")
            })
            .collect::<String>();
        for o in &outcomes {
            if let Some(raw) = o.raw {
                let first = o.first_beat.map(|v| v.to_string()).unwrap_or_default();
                text.push_str(&format!("{range}\t{}\t{raw}\t{first}\n", o.path.display()));
            }
        }
        std::fs::write(cache_path, text).expect("write BPM_REFERENCE_CACHE");
    }
    if let Some(out) = std::env::var_os("BPM_REFERENCE_OUT") {
        let mut csv = String::from(
            "reference,raw,final,error,peak,snap_ratio,family_margin,family_final,\
             rb_first_ms,rb_grid_bpm,stratum_first_ms,first_beat_ms,title,path\n",
        );
        for o in &outcomes {
            csv.push_str(&format!(
                "{:.2},{},{},{},{},{},{},{},{},{},{},{},\"{}\",\"{}\"\n",
                o.reference,
                o.raw.map(|v| format!("{v:.2}")).unwrap_or_default(),
                o.refined.map(|v| format!("{v:.2}")).unwrap_or_default(),
                octave_error(o.refined, o.reference)
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_default(),
                o.peak.map(|p| format!("{:.2}", p.0)).unwrap_or_default(),
                o.peak.map(|p| format!("{:.4}", p.1)).unwrap_or_default(),
                o.family.map(|f| format!("{:.4}", f.0)).unwrap_or_default(),
                o.family.map(|f| format!("{:.2}", f.1)).unwrap_or_default(),
                o.grid.map(|g| format!("{:.0}", g.0)).unwrap_or_default(),
                o.grid.map(|g| format!("{:.2}", g.1)).unwrap_or_default(),
                o.first_beat
                    .map(|v| format!("{:.1}", v * 1000.0))
                    .unwrap_or_default(),
                o.own_first_beat
                    .map(|v| format!("{:.1}", v * 1000.0))
                    .unwrap_or_default(),
                o.title.replace('"', "'"),
                o.path.display()
            ));
        }
        std::fs::write(out, csv).expect("write BPM_REFERENCE_OUT");
    }

    println!(
        "\nrange {range}, {} tracks; within N BPM of rekordbox, any octave",
        outcomes.len()
    );
    for (label, grid) in [
        ("whole/half reference", true),
        ("off-grid reference", false),
    ] {
        let group = outcomes
            .iter()
            .filter(|o| on_grid(o.reference) == grid)
            .collect::<Vec<_>>();
        println!("  {label} ({} tracks):", group.len());
        for limit in [0.02, 0.05, 0.1, 0.25, 1.0] {
            let within = |error: Option<f64>| error.is_some_and(|e| e <= limit + 1e-9);
            println!(
                "    <= {limit:<4}  stratum raw {:>4}   final {:>4}",
                group
                    .iter()
                    .filter(|o| within(octave_error(o.raw.map(f64::from), o.reference)))
                    .count(),
                group
                    .iter()
                    .filter(|o| within(octave_error(o.refined, o.reference)))
                    .count()
            );
        }
    }
    // First beat against rekordbox's grid, on tracks whose BPM matches it (or
    // its half or double): elsewhere the beat positions can't be compared.
    let beat_errors = |first: fn(&Outcome) -> Option<f64>| {
        outcomes
            .iter()
            .filter_map(|o| {
                let (rb_first_ms, rb_bpm) = o.grid?;
                let bpm = o.refined?;
                [0.5, 1.0, 2.0]
                    .iter()
                    .any(|k| (bpm * k - rb_bpm).abs() <= 0.05 * k + 0.02)
                    .then_some(())?;
                let period_ms = 60_000.0 / rb_bpm;
                let off = (first(o)? * 1000.0 - rb_first_ms).rem_euclid(period_ms);
                Some(off.min(period_ms - off))
            })
            .collect::<Vec<_>>()
    };
    let stratum_errors = beat_errors(|o| o.first_beat);
    let own_errors = beat_errors(|o| o.own_first_beat);
    if !own_errors.is_empty() {
        println!(
            "  first beat vs rekordbox's grid ({} tracks with its BPM):",
            own_errors.len()
        );
        for limit in [10.0, 20.0, 50.0] {
            let within = |errors: &[f64]| errors.iter().filter(|&&e| e <= limit).count();
            println!(
                "    <= {limit:<3} ms  stratum {:>4}   ours {:>4}",
                within(&stratum_errors),
                within(&own_errors)
            );
        }
    }
}
