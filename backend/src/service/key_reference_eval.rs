//! Key detection against annotated keys (opt-in, `#[ignore]`d).
//!
//! Each track is decoded the way analysis decodes it (one pass, waveform
//! included) and its key detected several ways, all with the vendored
//! stratum's `analyze_key`:
//!
//! - `whole_default` / `whole_cheap`: the first `BPM_KEY_MAX_SAMPLES` (today's
//!   input), default key settings / cheap ones (no time smoothing, no
//!   harmonic mask).
//! - `chunks_cheap`: `CHUNK_SECS` chunks of the whole track, voted by stratum's
//!   key clarity.
//! - `window_default` / `window_cheap`: the `KEY_WINDOW_SECS` window with the
//!   most mid-band energy in the waveform the same decode drew (`key_window`).
//! - `app`: the app's own analysis (`analyze_audio_core`): key and BPM
//!   estimate from the key window over the whole track, the BPM refined on
//!   the beat pieces (`decode_for_detection`).
//!
//! The BPM is detected twice, today's way (whole capped input, then
//! `refine_bpm`) and chunked (`analyze_bpm` per chunk, confidence vote seeding
//! `refine_bpm` over the whole track), to check the two agree.
//!
//! ```text
//! KEY_REFERENCE_LIST=/path/to/list.tsv \
//!   cargo test --release -p backend key_reference -- --ignored --nocapture
//! ```
//!
//! The list has one `path<TAB>key` per line; keys such as `F# Major`,
//! `Bb minor`, `Am` or `C#`. Optional: `KEY_REFERENCE_JOBS` (default 4) and
//! `KEY_REFERENCE_OUT` (per-track CSV; by default
//! `test-results/key-reference/<unix time>_<list name>.csv`).

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use stratum_dsp::{AnalysisConfig, Key};

use super::analysis::{
    BPM_KEY_MAX_SAMPLES, CoreRequest, analyze_audio_core, first_pass,
    waveform_detail_bins_for_duration,
};
use super::bpm_key::{
    AnalysisEngines, KEY_WINDOW_SECS, key_window, refine_bpm, stratum_config, stratum_key_config,
};

const BPM_MIN: u32 = 70;
const BPM_MAX: u32 = 180;
/// Length of one analysis chunk.
const CHUNK_SECS: usize = 180;
/// Decode cap for the chunked setups: the whole of any track short of a long
/// mix (about 60 minutes at 44.1 kHz).
const WHOLE_TRACK_MAX_SAMPLES: usize = 160_000_000;

const VARIANTS: [&str; 6] = [
    "whole_default",
    "whole_cheap",
    "chunks_cheap",
    "window_default",
    "window_cheap",
    "app",
];

fn cheap_config() -> AnalysisConfig {
    stratum_key_config(BPM_MIN, BPM_MAX)
}

/// `CHUNK_SECS` pieces of `len` samples. A last piece shorter than half a
/// chunk is left out, unless it is the only one.
fn chunk_ranges(len: usize, sample_rate: u32) -> Vec<Range<usize>> {
    let chunk = CHUNK_SECS * sample_rate as usize;
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < len {
        let end = (start + chunk).min(len);
        if end - start >= chunk / 2 || ranges.is_empty() {
            ranges.push(start..end);
        }
        start = end;
    }
    ranges
}

/// Today's BPM: stratum over the capped input, refined.
fn whole_bpm(samples: &[f32], rate: u32) -> Option<f64> {
    let r = stratum_dsp::analyze_bpm(samples, rate, stratum_config(BPM_MIN, BPM_MAX)).ok()?;
    (r.bpm > 0.0).then(|| refine_bpm(samples, rate, r.bpm, BPM_MIN, BPM_MAX))
}

/// stratum's BPM per chunk, voted by its confidence in 0.5 BPM bins; the
/// winning bin seeds `refine_bpm` over all of `samples`.
fn chunked_bpm(samples: &[f32], rate: u32) -> Option<f64> {
    let mut votes: HashMap<i64, f32> = HashMap::new();
    for range in chunk_ranges(samples.len(), rate) {
        let Ok(r) =
            stratum_dsp::analyze_bpm(&samples[range], rate, stratum_config(BPM_MIN, BPM_MAX))
        else {
            continue;
        };
        if r.bpm > 0.0 {
            *votes.entry((r.bpm * 2.0).round() as i64).or_default() += r.bpm_confidence.max(1e-3);
        }
    }
    let seed = votes
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(bin, _)| bin as f32 / 2.0)?;
    Some(refine_bpm(samples, rate, seed, BPM_MIN, BPM_MAX))
}

fn key_of(samples: &[f32], rate: u32, config: AnalysisConfig) -> Option<(Key, f32)> {
    let r = stratum_dsp::analyze_key(samples, rate, config).ok()?;
    (r.key_confidence > 0.0 || r.key_clarity > 0.0).then_some((r.key, r.key_clarity))
}

/// The key of each chunk, voted by stratum's key clarity.
fn chunked_key(samples: &[f32], rate: u32) -> Option<Key> {
    let mut votes: Vec<(Key, f32)> = Vec::new();
    for range in chunk_ranges(samples.len(), rate) {
        if let Some((key, clarity)) = key_of(&samples[range], rate, cheap_config()) {
            match votes.iter_mut().find(|(k, _)| *k == key) {
                Some(vote) => vote.1 += clarity.max(1e-4),
                None => votes.push((key, clarity.max(1e-4))),
            }
        }
    }
    // First-seen order breaks ties: max_by keeps the last maximum, so search reversed.
    votes
        .into_iter()
        .rev()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Root (0 = C) and minor flag.
fn parse_key(text: &str) -> Option<(u8, bool)> {
    let text = text.trim();
    let (root, minor) = if let Some((root, mode)) = text.split_once(' ') {
        match mode.trim().to_ascii_lowercase().as_str() {
            "major" | "maj" => (root, false),
            "minor" | "min" => (root, true),
            _ => return None,
        }
    } else if let Some(root) = text.strip_suffix('m') {
        (root, true)
    } else {
        (text, false)
    };
    let names = [
        ("C", 0),
        ("C#", 1),
        ("Db", 1),
        ("D", 2),
        ("D#", 3),
        ("Eb", 3),
        ("E", 4),
        ("F", 5),
        ("F#", 6),
        ("Gb", 6),
        ("G", 7),
        ("G#", 8),
        ("Ab", 8),
        ("A", 9),
        ("A#", 10),
        ("Bb", 10),
        ("B", 11),
    ];
    names
        .iter()
        .find(|(n, _)| *n == root)
        .map(|&(_, i)| (i, minor))
}

fn key_parts(key: Key) -> (u8, bool) {
    match key {
        Key::Major(i) => (i as u8 % 12, false),
        Key::Minor(i) => (i as u8 % 12, true),
    }
}

/// How `found` relates to `reference`: same, relative, parallel (same root,
/// other mode), fifth (either way, same mode) or other.
fn relation(found: (u8, bool), reference: (u8, bool)) -> &'static str {
    let ((r1, m1), (r2, m2)) = (found, reference);
    if found == reference {
        "same"
    } else if m1 != m2 {
        let (major, minor) = if m1 { (r2, r1) } else { (r1, r2) };
        if (major + 9) % 12 == minor {
            "relative"
        } else if r1 == r2 {
            "parallel"
        } else {
            "other"
        }
    } else if matches!((i16::from(r1) - i16::from(r2)).rem_euclid(12), 5 | 7) {
        "fifth"
    } else {
        "other"
    }
}

/// The MIREX key score: 1 same, 0.5 fifth, 0.3 relative, 0.2 parallel.
fn mirex(relation: &str) -> f64 {
    match relation {
        "same" => 1.0,
        "fifth" => 0.5,
        "relative" => 0.3,
        "parallel" => 0.2,
        _ => 0.0,
    }
}

fn key_name((root, minor): (u8, bool)) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", names[root as usize], if minor { "m" } else { "" })
}

struct Outcome {
    path: PathBuf,
    reference: (u8, bool),
    duration_s: f64,
    keys: Vec<Option<(u8, bool)>>,
    secs: Vec<f64>,
    window_start_s: f64,
    bpm_whole: Option<f64>,
    bpm_chunked: Option<f64>,
    bpm_app: Option<f64>,
}

fn analyze(path: &Path, reference: (u8, bool)) -> Option<Outcome> {
    let decoded = first_pass(
        path,
        &|_| WHOLE_TRACK_MAX_SAMPLES,
        waveform_detail_bins_for_duration,
    )
    .ok()?;
    let rate = decoded.sample_rate;
    let all = &decoded.head;
    let capped = &all[..all.len().min(BPM_KEY_MAX_SAMPLES)];
    let window_len = KEY_WINDOW_SECS * rate as usize;
    let mid = decoded
        .waveform
        .as_ref()
        .map(|w| w.mid_energy.as_slice())
        .unwrap_or(&[]);
    let window = key_window(mid, all.len(), window_len);

    let mut keys = Vec::new();
    let mut secs = Vec::new();
    let mut timed = |f: &mut dyn FnMut() -> Option<Key>| {
        let t = Instant::now();
        keys.push(f().map(key_parts));
        secs.push(t.elapsed().as_secs_f64());
    };
    timed(&mut || key_of(capped, rate, stratum_config(BPM_MIN, BPM_MAX)).map(|k| k.0));
    timed(&mut || key_of(capped, rate, cheap_config()).map(|k| k.0));
    timed(&mut || chunked_key(all, rate));
    timed(&mut || {
        key_of(&all[window.clone()], rate, stratum_config(BPM_MIN, BPM_MAX)).map(|k| k.0)
    });
    timed(&mut || key_of(&all[window.clone()], rate, cheap_config()).map(|k| k.0));
    let mut bpm_app = None;
    let t = Instant::now();
    let app = analyze_audio_core(
        path,
        &CoreRequest {
            kept_bpm: None,
            kept_first_beat_ms: None,
            existing_first_beat_ms: None,
            detect_key: true,
            detection_required: false,
            bpm_min: BPM_MIN,
            bpm_max: BPM_MAX,
            engines: AnalysisEngines::STRATUM,
        },
        &mut |_, _| {},
    );
    if let Ok(app) = app {
        bpm_app = app.bpm;
        keys.push(app.key.as_deref().and_then(parse_key));
    } else {
        keys.push(None);
    }
    secs.push(t.elapsed().as_secs_f64());

    Some(Outcome {
        path: path.to_path_buf(),
        reference,
        duration_s: all.len() as f64 / f64::from(rate),
        keys,
        secs,
        window_start_s: window.start as f64 / f64::from(rate),
        bpm_whole: whole_bpm(capped, rate),
        bpm_chunked: chunked_bpm(all, rate),
        bpm_app,
    })
}

fn load_list(path: &Path) -> Vec<(PathBuf, (u8, bool))> {
    std::fs::read_to_string(path)
        .expect("read KEY_REFERENCE_LIST")
        .lines()
        .filter_map(|line| {
            let (file, key) = line.split_once('\t')?;
            Some((PathBuf::from(file), parse_key(key)?))
        })
        .collect()
}

#[test]
#[ignore]
fn key_reference() {
    let Some(list) = std::env::var_os("KEY_REFERENCE_LIST").map(PathBuf::from) else {
        println!("KEY_REFERENCE_LIST not set; skipping");
        return;
    };
    let references = load_list(&list);
    let jobs = std::env::var("KEY_REFERENCE_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4usize);
    let next = AtomicUsize::new(0);
    let outcomes = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, reference)) = references.get(i) else {
                        break;
                    };
                    match analyze(path, *reference) {
                        Some(o) => {
                            eprintln!("{}/{} {}", i + 1, references.len(), path.display());
                            outcomes.lock().unwrap().push(o);
                        }
                        None => eprintln!("FAILED to decode {}", path.display()),
                    }
                }
            });
        }
    });
    let mut outcomes = outcomes.into_inner().unwrap();
    outcomes.sort_by(|a, b| a.path.cmp(&b.path));

    let out = std::env::var_os("KEY_REFERENCE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let name = list.file_stem().map(|s| s.to_string_lossy().into_owned());
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test-results/key-reference")
                .join(format!("{stamp}_{}.csv", name.unwrap_or_default()))
        });
    let mut csv = format!(
        "reference,duration_s,window_start_s,{},bpm_whole,bpm_chunked,bpm_app,path\n",
        VARIANTS.join(",")
    );
    for o in &outcomes {
        let keys: Vec<String> = o
            .keys
            .iter()
            .map(|k| k.map(key_name).unwrap_or_default())
            .collect();
        csv.push_str(&format!(
            "{},{:.1},{:.1},{},{},{},{},\"{}\"\n",
            key_name(o.reference),
            o.duration_s,
            o.window_start_s,
            keys.join(","),
            o.bpm_whole.map(|v| format!("{v:.2}")).unwrap_or_default(),
            o.bpm_chunked.map(|v| format!("{v:.2}")).unwrap_or_default(),
            o.bpm_app.map(|v| format!("{v:.2}")).unwrap_or_default(),
            o.path.display()
        ));
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("create the CSV's folder");
    }
    std::fs::write(&out, csv).expect("write KEY_REFERENCE_OUT");
    println!("per-track CSV: {}", out.display());

    let n = outcomes.len();
    println!("\n{n} tracks against the reference keys");
    println!("variant          same  fifth  relative  parallel  other  none  MIREX  key s/track");
    for (v, name) in VARIANTS.iter().enumerate() {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut score = 0.0;
        for o in &outcomes {
            let rel = o.keys[v]
                .map(|k| relation(k, o.reference))
                .unwrap_or("none");
            *counts.entry(rel).or_default() += 1;
            score += mirex(rel);
        }
        let mean_secs = outcomes.iter().map(|o| o.secs[v]).sum::<f64>() / n.max(1) as f64;
        let c = |k| counts.get(k).copied().unwrap_or(0);
        println!(
            "{name:<16} {:>4}  {:>5}  {:>8}  {:>8}  {:>5}  {:>4}  {:>5.1}%  {mean_secs:.2}",
            c("same"),
            c("fifth"),
            c("relative"),
            c("parallel"),
            c("other"),
            c("none"),
            100.0 * score / n.max(1) as f64
        );
    }
    println!("\nsame key as whole_default:");
    for (v, name) in VARIANTS.iter().enumerate().skip(1) {
        let same = outcomes.iter().filter(|o| o.keys[v] == o.keys[0]).count();
        println!("  {name:<16} {same}/{n}");
    }
    let bpm_same = outcomes
        .iter()
        .filter(|o| match (o.bpm_whole, o.bpm_chunked) {
            (Some(a), Some(b)) => (a - b).abs() < 0.005,
            (None, None) => true,
            _ => false,
        })
        .count();
    let app_same = outcomes.iter().filter(|o| o.bpm_app == o.bpm_whole).count();
    println!("\napp BPM same as whole (capped) BPM: {app_same}/{n}");
    println!("\nchunked BPM same as whole: {bpm_same}/{n}");
    for o in outcomes
        .iter()
        .filter(|o| match (o.bpm_whole, o.bpm_chunked) {
            (Some(a), Some(b)) => (a - b).abs() >= 0.005,
            (None, None) => false,
            _ => true,
        })
    {
        println!(
            "  {:?} vs {:?}  {}",
            o.bpm_whole,
            o.bpm_chunked,
            o.path.display()
        );
    }
}

/// Peak memory (VmHWM) of one setup on one file, decoded the way analysis
/// decodes it (`BPM_KEY_MAX_SAMPLES`). Run each setup in its own process:
/// the allocator keeps what an earlier setup freed, so later peaks would
/// look too small. Linux only.
///
/// ```text
/// MEM_FILE=<audio> MEM_SETUP=today|bpm|key_window|bpm_key_window \
///   <test binary> --ignored --exact service::key_reference_eval::key_memory
/// ```
///
/// Prints `MEM|setup|seconds|MB after decode|peak MB|BPM|key|file`.
#[test]
#[ignore]
fn key_memory() {
    fn hwm_mb() -> i64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find(|l| l.starts_with("VmHWM"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<i64>().ok())
            .unwrap_or(0)
            / 1024
    }
    let (Some(file), Ok(setup)) = (std::env::var_os("MEM_FILE"), std::env::var("MEM_SETUP")) else {
        println!("MEM_FILE / MEM_SETUP not set; skipping");
        return;
    };
    let path = PathBuf::from(file);
    let decoded = first_pass(
        &path,
        &|_| BPM_KEY_MAX_SAMPLES,
        waveform_detail_bins_for_duration,
    )
    .expect("decode");
    let after_decode = hwm_mb();
    let (samples, rate) = (&decoded.head, decoded.sample_rate);
    let mid = decoded
        .waveform
        .as_ref()
        .map(|w| w.mid_energy.as_slice())
        .unwrap_or(&[]);
    let window = || {
        let total = decoded
            .duration_ms
            .map(|ms| (ms * u64::from(rate) / 1000) as usize)
            .unwrap_or(samples.len());
        let w = key_window(mid, total, KEY_WINDOW_SECS * rate as usize);
        // The window lies in the decoded part (today's cap); past it, take the last part decoded.
        let end = w.end.min(samples.len());
        end.saturating_sub(w.len())..end
    };
    let t = Instant::now();
    let (bpm, key) = match setup.as_str() {
        "today" => {
            let r = stratum_dsp::analyze_audio(samples, rate, stratum_config(BPM_MIN, BPM_MAX))
                .expect("stratum");
            (
                Some(refine_bpm(samples, rate, r.bpm, BPM_MIN, BPM_MAX)),
                Some(r.key),
            )
        }
        "bpm" => (whole_bpm(samples, rate), None),
        "key_window" => (
            None,
            key_of(&samples[window()], rate, cheap_config()).map(|k| k.0),
        ),
        "bpm_key_window" => (
            whole_bpm(samples, rate),
            key_of(&samples[window()], rate, cheap_config()).map(|k| k.0),
        ),
        other => panic!("unknown MEM_SETUP {other}"),
    };
    println!(
        "MEM|{setup}|{:.1}|{after_decode}|{}|{}|{}|{}",
        t.elapsed().as_secs_f64(),
        hwm_mb(),
        bpm.map(|b| format!("{b:.2}")).unwrap_or_default(),
        key.map(|k| k.name()).unwrap_or_default(),
        path.display()
    );
}

/// The analysis job's memory per worker, for `ANALYSIS_MEMORY_PER_WORKER_*`:
/// a fresh data folder, `MEM_BATCH_ROOT` added as a source folder (indexed as
/// the app does), then its tracks analyzed in one job with
/// `MEM_BATCH_WORKERS` workers (default 14): all of them, or with
/// `MEM_BATCH_LIMIT`, that many: the 20 largest files (the longest tracks,
/// which need the most) and the rest spread evenly over the others by size. A thread
/// samples the process's resident memory every second. Linux only; build
/// with `--release`.
///
/// ```text
/// MEM_BATCH_ROOT=/path/to/music MEM_BATCH_WORKERS=14 [MEM_BATCH_LIMIT=200] \
///   cargo test --release -p backend --lib analysis_memory_batch -- --ignored --nocapture
/// ```
///
/// Prints the samples (`RSS|seconds|MB`), then the peak and peak per worker.
#[test]
#[ignore]
fn analysis_memory_batch() {
    use crate::commands::BackendCommands;
    use crate::models::{
        AddSourceFoldersRequest, AnalyzeNewTracksRequest, BrowseSourceFilesRequest,
    };
    use std::sync::atomic::AtomicBool;

    fn status_mb(field: &str) -> i64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find(|l| l.starts_with(field))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<i64>().ok())
            .unwrap_or(0)
            / 1024
    }
    let Some(root) = std::env::var_os("MEM_BATCH_ROOT").map(PathBuf::from) else {
        println!("MEM_BATCH_ROOT not set; skipping");
        return;
    };
    let workers = std::env::var("MEM_BATCH_WORKERS").unwrap_or_else(|_| "14".to_string());
    // SAFETY: this ignored test runs alone (`--exact` / a filter that picks
    // only it); nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", &workers) };
    let workers: f64 = workers.parse().expect("MEM_BATCH_WORKERS is a number");

    let data = tempfile::tempdir().expect("temp data folder");
    let backend = BackendCommands::new(data.path()).expect("backend");
    let added = backend.add_source_folders(AddSourceFoldersRequest {
        paths: vec![root.to_string_lossy().into_owned()],
    });
    assert!(added.ok, "add source folder: {:?}", added.error);
    let limit = std::env::var("MEM_BATCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());
    let track_ids = limit.map(|limit| {
        let listed = backend.browse_source_files(BrowseSourceFilesRequest {
            limit: usize::MAX / 2,
            ..Default::default()
        });
        let mut tracks = listed.data.expect("library tracks").items;
        tracks.sort_by_key(|t| std::cmp::Reverse(t.file_size_bytes.unwrap_or(0)));
        let largest = tracks.len().min(20).min(limit);
        let rest = tracks.split_off(largest);
        let wanted = limit - largest;
        let stride = (rest.len() as f64 / wanted.max(1) as f64).max(1.0);
        tracks
            .into_iter()
            .chain((0..wanted.min(rest.len())).map(|i| rest[(i as f64 * stride) as usize].clone()))
            .map(|t| t.id)
            .collect::<Vec<_>>()
    });
    let baseline = status_mb("VmRSS");

    let done = AtomicBool::new(false);
    let start = Instant::now();
    let samples = Mutex::new(Vec::new());
    let (analyzed, failed) = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                let mb = status_mb("VmRSS");
                samples
                    .lock()
                    .unwrap()
                    .push((start.elapsed().as_secs(), mb));
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
        let resp = backend.analyze_new_tracks(match &track_ids {
            Some(ids) => AnalyzeNewTracksRequest {
                track_ids: ids.clone(),
                ..Default::default()
            },
            None => AnalyzeNewTracksRequest {
                scope_to_library_filter: true,
                ..Default::default()
            },
        });
        done.store(true, Ordering::Relaxed);
        let data = resp.data.expect("analysis result");
        (data.analyzed, data.failed)
    });
    let secs = start.elapsed().as_secs_f64();
    for (t, mb) in samples.into_inner().unwrap() {
        println!("RSS|{t}|{mb}");
    }
    let peak = status_mb("VmHWM");
    println!(
        "\n{analyzed} analyzed, {failed} failed in {secs:.0} s with {workers} workers; \
         baseline {baseline} MB, peak {peak} MB, peak per worker {:.0} MB",
        (peak - baseline) as f64 / workers
    );
}

/// A key-settings variant: a name and the change it makes to the app's key
/// settings (`stratum_key_config`).
type Change = fn(&mut AnalysisConfig);
type Variant = (&'static str, Change);

/// The key settings `key_sweep` tries, each one change from the app's
/// (`stratum_key_config`); some only undo or redo what the app's already set.
fn sweep_variants() -> Vec<Variant> {
    use stratum_dsp::features::key::templates::TemplateSet;
    vec![
        ("app", |_| {}),
        ("smooth+mask", |c| {
            c.enable_key_spectrogram_time_smoothing = true;
            c.enable_key_harmonic_mask = true;
        }),
        ("smooth", |c| c.enable_key_spectrogram_time_smoothing = true),
        ("mask", |c| c.enable_key_harmonic_mask = true),
        ("hpss", |c| c.enable_key_hpss_harmonic = true),
        ("mode_heur", |c| c.enable_key_mode_heuristic = true),
        ("mode_heur_strict", |c| {
            c.enable_key_mode_heuristic = true;
            c.key_mode_third_ratio_margin = 0.05;
            c.key_mode_flip_min_score_ratio = 0.92;
        }),
        ("minor_bonus_0.2", |c| {
            c.enable_key_minor_harmonic_bonus = true
        }),
        ("minor_bonus_0.5", |c| {
            c.enable_key_minor_harmonic_bonus = true;
            c.key_minor_leading_tone_bonus_weight = 0.5;
        }),
        ("minor_bonus_0.8", |c| {
            c.enable_key_minor_harmonic_bonus = true;
            c.key_minor_leading_tone_bonus_weight = 0.8;
        }),
        ("temperley", |c| c.key_template_set = TemplateSet::Temperley),
        ("ensemble", |c| c.enable_key_ensemble = true),
        ("tuning", |c| c.enable_key_tuning_compensation = true),
        ("edge_trim", |c| c.enable_key_edge_trim = true),
        ("whitening", |c| c.enable_key_hpcp_whitening = true),
        ("bass_blend", |c| c.enable_key_hpcp_bass_blend = true),
        ("no_hpcp", |c| c.enable_key_hpcp = false),
        ("no_seg_vote", |c| c.enable_key_segment_voting = false),
        ("median", |c| c.enable_key_median = true),
        ("multi_scale", |c| c.enable_key_multi_scale = true),
        ("log_freq", |c| c.enable_key_log_frequency = true),
        ("no_frame_weight", |c| c.enable_key_frame_weighting = false),
        ("tonal_pow_1", |c| c.key_tonalness_power = 1.0),
        ("tonal_pow_3", |c| c.key_tonalness_power = 3.0),
        ("energy_pow_0", |c| c.key_energy_power = 0.0),
        ("energy_pow_1", |c| c.key_energy_power = 1.0),
        ("stft_4096", |c| c.key_stft_frame_size = 4096),
        ("stft_16384", |c| c.key_stft_frame_size = 16384),
        ("hpcp_peaks_12", |c| c.key_hpcp_peaks_per_frame = 12),
        ("hpcp_peaks_48", |c| c.key_hpcp_peaks_per_frame = 48),
        ("hpcp_harm_1", |c| c.key_hpcp_num_harmonics = 1),
        ("hpcp_harm_8", |c| c.key_hpcp_num_harmonics = 8),
        ("hpcp_decay_0.8", |c| c.key_hpcp_harmonic_decay = 0.8),
        ("hpcp_mag_1", |c| c.key_hpcp_mag_power = 1.0),
        ("hpcp_mag_0.25", |c| c.key_hpcp_mag_power = 0.25),
        ("seg_len_512", |c| c.key_segment_len_frames = 512),
        ("seg_len_2048", |c| {
            c.key_segment_len_frames = 2048;
            c.key_segment_hop_frames = 1024;
        }),
        ("seg_clarity_0", |c| c.key_segment_min_clarity = 0.0),
        ("seg_clarity_0.4", |c| c.key_segment_min_clarity = 0.4),
        ("bass_w_0.2", |c| c.key_hpcp_bass_weight = 0.2),
        ("bass_w_0.5", |c| c.key_hpcp_bass_weight = 0.5),
        ("bass_fmax_150", |c| c.key_hpcp_bass_fmax_hz = 150.0),
        ("ms_short", |c| {
            c.key_multi_scale_lengths = vec![60, 120, 360]
        }),
        ("ms_long", |c| {
            c.key_multi_scale_lengths = vec![360, 720, 1440]
        }),
        ("ms_clarity_0.1", |c| c.key_multi_scale_min_clarity = 0.1),
        ("ms_clarity_0.3", |c| c.key_multi_scale_min_clarity = 0.3),
        ("tuning_0.2", |c| c.key_tuning_max_abs_semitones = 0.2),
    ]
}

/// Key settings against annotated keys: each track decoded once, its key
/// window (as the app picks it) run through every `sweep_variants` setting.
///
/// ```text
/// KEY_REFERENCE_LIST=/path/to/list.tsv [KEY_SWEEP_ONLY=mask,temperley+tuning] \
///   cargo test --release -p backend key_sweep -- --ignored --nocapture
/// ```
///
/// `KEY_SWEEP_ONLY` picks variants by name; `a+b` applies both. Prints the
/// relations and MIREX score of each, how many tracks it gets right that the
/// app's settings get wrong and the other way round, and its key time.
#[test]
#[ignore]
fn key_sweep() {
    let Some(list) = std::env::var_os("KEY_REFERENCE_LIST").map(PathBuf::from) else {
        println!("KEY_REFERENCE_LIST not set; skipping");
        return;
    };
    let singles = sweep_variants();
    let names: Vec<String> = match std::env::var("KEY_SWEEP_ONLY") {
        Ok(v) => std::iter::once("app".to_string())
            .chain(v.split(',').map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => singles.iter().map(|(n, _)| n.to_string()).collect(),
    };
    // Each variant: its name and the changes it makes, in order.
    let variants: Vec<(String, Vec<Change>)> = names
        .iter()
        .map(|name| {
            let changes = name
                .split('+')
                .map(|part| {
                    singles
                        .iter()
                        .find(|(n, _)| *n == part)
                        .unwrap_or_else(|| panic!("no key sweep variant {part}"))
                        .1
                })
                .collect();
            (name.clone(), changes)
        })
        .collect();
    let references = load_list(&list);
    let jobs = std::env::var("KEY_REFERENCE_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4usize);
    let next = AtomicUsize::new(0);
    // Per track: the reference and each variant's key.
    type Keys = Vec<(Option<(u8, bool)>, f64)>;
    let results: Mutex<Vec<((u8, bool), Keys)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, reference)) = references.get(i) else {
                        break;
                    };
                    let Ok(decoded) = first_pass(
                        path,
                        &|_| BPM_KEY_MAX_SAMPLES,
                        waveform_detail_bins_for_duration,
                    ) else {
                        eprintln!("FAILED to decode {}", path.display());
                        continue;
                    };
                    let rate = decoded.sample_rate;
                    let mid = decoded
                        .waveform
                        .as_ref()
                        .map(|w| w.mid_energy.as_slice())
                        .unwrap_or(&[]);
                    let window =
                        key_window(mid, decoded.head.len(), KEY_WINDOW_SECS * rate as usize);
                    let samples = &decoded.head[window];
                    let keys = variants
                        .iter()
                        .map(|(_, changes)| {
                            let mut config = cheap_config();
                            changes.iter().for_each(|change| change(&mut config));
                            let t = Instant::now();
                            let key = key_of(samples, rate, config).map(|k| key_parts(k.0));
                            (key, t.elapsed().as_secs_f64())
                        })
                        .collect();
                    eprintln!("{}/{}", i + 1, references.len());
                    results.lock().unwrap().push((*reference, keys));
                }
            });
        }
    });
    let results = results.into_inner().unwrap();
    let n = results.len();
    println!(
        "\n{n} tracks against the reference keys ({})",
        list.display()
    );
    println!(
        "variant            same  fifth  relat  paral  other  none   MIREX   +won  -lost  s/track"
    );
    for (v, (name, _)) in variants.iter().enumerate() {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let (mut score, mut won, mut lost, mut secs) = (0.0, 0, 0, 0.0);
        for (reference, keys) in &results {
            let rel = keys[v].0.map(|k| relation(k, *reference)).unwrap_or("none");
            *counts.entry(rel).or_default() += 1;
            score += mirex(rel);
            secs += keys[v].1;
            let app_right = keys[0].0 == Some(*reference);
            let right = keys[v].0 == Some(*reference);
            won += usize::from(right && !app_right);
            lost += usize::from(app_right && !right);
        }
        let c = |k| counts.get(k).copied().unwrap_or(0);
        println!(
            "SWEEP|{name:<17} {:>4}  {:>5}  {:>5}  {:>5}  {:>5}  {:>4}  {:>5.1}%  {won:>5}  {lost:>5}  {:.2}",
            c("same"),
            c("fifth"),
            c("relative"),
            c("parallel"),
            c("other"),
            c("none"),
            100.0 * score / n.max(1) as f64,
            secs / n.max(1) as f64
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reference_key_spellings() {
        assert_eq!(parse_key("F# Major"), Some((6, false)));
        assert_eq!(parse_key("Bb minor"), Some((10, true)));
        assert_eq!(parse_key("A#m"), Some((10, true)));
        assert_eq!(parse_key("C"), Some((0, false)));
        assert_eq!(parse_key("H minor"), None);
    }

    #[test]
    fn relations_follow_mirex() {
        let am = (9, true);
        assert_eq!(relation(am, am), "same");
        assert_eq!(relation((0, false), am), "relative");
        assert_eq!(relation((9, false), am), "parallel");
        assert_eq!(relation((4, true), am), "fifth");
        assert_eq!(relation((2, true), am), "fifth");
        assert_eq!(relation((1, true), am), "other");
    }

    #[test]
    fn short_tail_chunk_is_left_out() {
        let rate = 10;
        let chunk = CHUNK_SECS * rate as usize;
        assert_eq!(
            chunk_ranges(chunk * 2 + chunk / 4, rate),
            vec![0..chunk, chunk..chunk * 2]
        );
        assert_eq!(
            chunk_ranges(chunk * 2 + chunk / 2, rate),
            vec![0..chunk, chunk..chunk * 2, chunk * 2..chunk * 2 + chunk / 2]
        );
        assert_eq!(chunk_ranges(chunk / 4, rate), vec![0..chunk / 4]);
    }
}
