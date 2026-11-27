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
//! - `window_default` / `window_cheap`: the `CHUNK_SECS` window with the most
//!   mid-band energy in the waveform the same decode drew.
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
    BPM_KEY_MAX_SAMPLES, decode_audio_with_waveform, waveform_detail_bins_for_duration,
};
use super::bpm_key::{refine_bpm, stratum_config};

const BPM_MIN: u32 = 70;
const BPM_MAX: u32 = 180;
/// Length of one analysis chunk and of the key window.
const CHUNK_SECS: usize = 180;
/// Decode cap for the chunked setups: the whole of any track short of a long
/// mix (about 60 minutes at 44.1 kHz).
const WHOLE_TRACK_MAX_SAMPLES: usize = 160_000_000;

const VARIANTS: [&str; 5] = [
    "whole_default",
    "whole_cheap",
    "chunks_cheap",
    "window_default",
    "window_cheap",
];

fn cheap_config() -> AnalysisConfig {
    AnalysisConfig {
        enable_key_spectrogram_time_smoothing: false,
        enable_key_harmonic_mask: false,
        ..stratum_config(BPM_MIN, BPM_MAX)
    }
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

/// The `window` samples (of `total`) whose waveform bins hold the most
/// mid-band energy; the earliest such window on a tie. The bins spread
/// evenly over the track.
fn key_window(mid_energy: &[u8], total: usize, window: usize) -> Range<usize> {
    let bins = mid_energy.len();
    if total <= window || bins == 0 {
        return 0..total.min(window);
    }
    let width = (window * bins).div_ceil(total).clamp(1, bins);
    let mut sum: u64 = mid_energy[..width].iter().map(|&v| u64::from(v)).sum();
    let (mut best, mut best_start) = (sum, 0);
    for start in 1..=bins - width {
        sum = sum + u64::from(mid_energy[start + width - 1]) - u64::from(mid_energy[start - 1]);
        if sum > best {
            (best, best_start) = (sum, start);
        }
    }
    let start = (best_start * total / bins).min(total - window);
    start..start + window
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
}

fn analyze(path: &Path, reference: (u8, bool)) -> Option<Outcome> {
    let decoded = decode_audio_with_waveform(
        path,
        WHOLE_TRACK_MAX_SAMPLES,
        waveform_detail_bins_for_duration,
    )
    .ok()?;
    let rate = decoded.sample_rate;
    let all = &decoded.samples;
    let capped = &all[..all.len().min(BPM_KEY_MAX_SAMPLES)];
    let window_len = CHUNK_SECS * rate as usize;
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

    Some(Outcome {
        path: path.to_path_buf(),
        reference,
        duration_s: all.len() as f64 / f64::from(rate),
        keys,
        secs,
        window_start_s: window.start as f64 / f64::from(rate),
        bpm_whole: whole_bpm(capped, rate),
        bpm_chunked: chunked_bpm(all, rate),
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
        "reference,duration_s,window_start_s,{},bpm_whole,bpm_chunked,path\n",
        VARIANTS.join(",")
    );
    for o in &outcomes {
        let keys: Vec<String> = o
            .keys
            .iter()
            .map(|k| k.map(key_name).unwrap_or_default())
            .collect();
        csv.push_str(&format!(
            "{},{:.1},{:.1},{},{},{},\"{}\"\n",
            key_name(o.reference),
            o.duration_s,
            o.window_start_s,
            keys.join(","),
            o.bpm_whole.map(|v| format!("{v:.2}")).unwrap_or_default(),
            o.bpm_chunked.map(|v| format!("{v:.2}")).unwrap_or_default(),
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
    let decoded = decode_audio_with_waveform(
        &path,
        BPM_KEY_MAX_SAMPLES,
        waveform_detail_bins_for_duration,
    )
    .expect("decode");
    let after_decode = hwm_mb();
    let (samples, rate) = (&decoded.samples, decoded.sample_rate);
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
        let w = key_window(mid, total, CHUNK_SECS * rate as usize);
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

    #[test]
    fn key_window_takes_the_most_mid_energy() {
        // 10 bins over 1000 samples; a 300-sample window spans 3 bins.
        let mid = [1, 1, 1, 1, 9, 9, 9, 1, 1, 1];
        assert_eq!(key_window(&mid, 1000, 300), 400..700);
        // A tie keeps the earliest; the window never runs past the end.
        assert_eq!(key_window(&[5; 10], 1000, 300), 0..300);
        assert_eq!(
            key_window(&[0, 0, 0, 0, 0, 0, 0, 0, 9, 9], 1000, 300),
            700..1000
        );
        assert_eq!(key_window(&mid, 200, 300), 0..200);
    }
}
