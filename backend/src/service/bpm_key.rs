//! BPM/key analysis engine abstraction.
//!
//! Routes analysis to either the built-in stratum-dsp engine (pure Rust, no
//! external runtime) or the legacy essentia.js engine (requires Node.js).

use crate::error::{BackendError, BackendResult};

/// Which BPM/key detection engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisEngine {
    /// Built-in pure-Rust engine (stratum-dsp). No external runtime required.
    Stratum,
    /// Legacy essentia.js via Node.js shell-out.
    Essentia,
}

impl AnalysisEngine {
    /// Parse from setting string. Unknown values fall back to `Stratum`.
    pub fn from_setting(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "essentia" => Self::Essentia,
            _ => Self::Stratum,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stratum => "stratum",
            Self::Essentia => "essentia",
        }
    }
}

/// BPM/key detection result (engine-agnostic).
pub struct BpmKeyResult {
    pub bpm: Option<f64>,
    pub key: Option<String>,
    /// First beat (downbeat) position in milliseconds, from beat-grid analysis.
    pub first_beat_ms: Option<u32>,
}

fn empty_bpm_key_result() -> BpmKeyResult {
    BpmKeyResult {
        bpm: None,
        key: None,
        first_beat_ms: None,
    }
}

/// Run BPM/key detection using the stratum-dsp engine.
pub fn detect_bpm_key_stratum(
    samples: &[f32],
    sample_rate: u32,
    bpm_min: u32,
    bpm_max: u32,
) -> BackendResult<BpmKeyResult> {
    if samples.is_empty() || sample_rate == 0 {
        return Ok(empty_bpm_key_result());
    }

    let result = stratum_dsp::analyze_audio(samples, sample_rate, stratum_config(bpm_min, bpm_max))
        .map_err(|e| BackendError::Internal(format!("stratum-dsp analysis failed: {e}")))?;

    let bpm =
        (result.bpm > 0.0).then(|| refine_bpm(samples, sample_rate, result.bpm, bpm_min, bpm_max));

    let key = (!result.key.name().is_empty()).then(|| result.key.name());

    // stratum places its beats at its own estimate, which `refine_bpm` may
    // have moved by up to 2 BPM or to a 3:2 / 4:3 relative; its first beat
    // is also often well off a beat.
    let first_beat_ms = bpm
        .and_then(|bpm| first_beat_secs(samples, sample_rate, bpm))
        .or_else(|| result.beat_grid.beats.first().map(|&t| f64::from(t)))
        .map(|t| (t * 1000.0).round() as u32);

    Ok(BpmKeyResult {
        bpm,
        key,
        first_beat_ms,
    })
}

pub(super) fn stratum_config(bpm_min: u32, bpm_max: u32) -> stratum_dsp::AnalysisConfig {
    stratum_dsp::AnalysisConfig {
        min_bpm: bpm_min as f32,
        max_bpm: bpm_max as f32,
        ..Default::default()
    }
}

/// Hop between onset-envelope frames, in samples.
const ONSET_HOP: usize = 512;
/// How far around stratum's estimate to look: its typical error is ±2 BPM.
const BPM_SEARCH_RADIUS: f64 = 2.0;
/// Resolution of the search; rekordbox stores BPMs to 0.01.
const BPM_SEARCH_STEP: f64 = 0.01;
/// The best tempo's beat coherence must reach this, and beat every tempo at
/// least `RIVAL_DISTANCE` away by `MIN_COHERENCE_LEAD`, to be trusted over
/// plain rounding.
const MIN_BEAT_COHERENCE: f64 = 0.06;
const MIN_COHERENCE_LEAD: f64 = 1.5;
const RIVAL_DISTANCE: f64 = 0.5;
/// A tempo is scored on the beat and on its 2× and 4× subdivisions. At the
/// beat alone, a backbeat half a beat off cancels part of the score and
/// blunts the peak (a drum & bass snare when stratum answers 87 for 174);
/// on the subdivisions it lands in phase, and the peak gets sharper.
const SUBDIVISIONS: [f64; 3] = [1.0, 2.0, 4.0];
/// The nearest whole or half BPM is kept when it is this coherent relative
/// to the best tempo: on a track whose tempo is a whole or half BPM, the
/// best tempo can still land a few hundredths off it.
const GRID_SNAP_COHERENCE: f64 = 0.9;
/// stratum sometimes answers a tempo related to the real one by 3:2 or 4:3
/// (83 for 125, 116 for 174). At such a tempo the beats land on two or
/// three different phases and cancel out, so the real one scores clearly
/// higher. Halving and doubling are left alone: there, every onset still
/// lands on the grid and the score can't tell which is right.
///
/// Each factor comes with how many times more coherent the related tempo
/// must be than stratum's to replace it. 3/2 and 3/4 need far more: one of
/// their `SUBDIVISIONS` is 3× stratum's tempo, where every one of its beats
/// lands in phase too, so they score well even when they are wrong.
const TEMPO_FAMILY: [(f64, f64); 4] = [(1.5, 8.0), (0.75, 8.0), (2.0 / 3.0, 2.5), (4.0 / 3.0, 2.5)];

/// Turn stratum's raw estimate into the BPM stored for the track.
///
/// stratum searches whole-BPM steps and is typically off by up to ±2, so its
/// decimals mean nothing on their own. Instead, every tempo within ±2 of it
/// is tried, in 0.01 steps, against the whole track: a tempo even 0.1 BPM off
/// drifts a beat over ten minutes, so only the right one keeps the onsets in
/// phase. That finds tempos between whole and half values (174.79) as well
/// as whole and half ones. When no tempo clearly wins, the whole and half
/// BPMs alone are compared the same way; when none of those wins either
/// (ambiguous rhythm, tempo changes), the estimate is rounded to a whole BPM.
/// stratum's octave is kept as is, but a tempo related to it by 3:2 or 4:3
/// replaces it when that one clearly fits the track better (`TEMPO_FAMILY`).
pub(super) fn refine_bpm(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> f64 {
    let estimate = match tempo_family_alternative(samples, sample_rate, raw, bpm_min, bpm_max) {
        Some((alternative, margin)) if margin >= 1.0 => alternative,
        _ => raw,
    };
    refine_near_estimate(samples, sample_rate, estimate, bpm_min, bpm_max)
}

/// `refine_bpm` without the 3:2 / 4:3 check.
pub(super) fn refine_near_estimate(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> f64 {
    match find_tempo_peak(samples, sample_rate, raw, bpm_min, bpm_max) {
        None => f64::from(raw.round()),
        Some(peak) if peak.grid_coherence >= GRID_SNAP_COHERENCE * peak.coherence => peak.grid,
        Some(peak) => (peak.bpm * 100.0).round() / 100.0,
    }
}

/// The tempo related to stratum's estimate by 3:2 or 4:3 (`TEMPO_FAMILY`)
/// that comes closest to replacing it, and how close: its coherence lead
/// over the estimate divided by the lead its factor requires, so 1.0 or
/// more means it should replace the estimate. Both are compared at their
/// best whole or half BPM within the search radius, which is enough to
/// tell tempo families apart.
pub(super) fn tempo_family_alternative(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<(f32, f64)> {
    let envelope = onset_envelope(samples);
    let frame_secs = ONSET_HOP as f64 / f64::from(sample_rate);
    let (min, max) = (f64::from(bpm_min), f64::from(bpm_max));
    let raw = f64::from(raw);
    let best_near = |centre: f64, radius: f64| {
        let steps = (radius * 2.0).round() as i32;
        (-steps..=steps)
            .map(|step| (centre * 2.0).round() / 2.0 + f64::from(step) * 0.5)
            .filter(|&bpm| bpm >= min && bpm <= max)
            .map(|bpm| subdivided_coherence(&envelope, frame_secs, bpm))
            .fold(0.0, f64::max)
    };
    let own = best_near(raw, BPM_SEARCH_RADIUS);
    if own <= 0.0 {
        return None;
    }
    TEMPO_FAMILY
        .iter()
        .map(|&(factor, required)| (raw * factor, factor, required))
        .filter(|&(centre, _, _)| centre >= min && centre <= max)
        .map(|(centre, factor, required)| {
            // stratum's error scales with the factor.
            let score = best_near(centre, BPM_SEARCH_RADIUS * factor.max(1.0));
            (centre as f32, score / own / required)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// The tempo that clearly keeps the onsets in phase best, near stratum's
/// estimate, and the nearest whole or half BPM to it.
pub(super) struct TempoPeak {
    pub bpm: f64,
    pub coherence: f64,
    pub grid: f64,
    pub grid_coherence: f64,
}

/// `None` when no tempo clearly wins, not even among whole and half BPMs.
pub(super) fn find_tempo_peak(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<TempoPeak> {
    let envelope = onset_envelope(samples);
    let frame_secs = ONSET_HOP as f64 / f64::from(sample_rate);
    let coherence = |bpm: f64| subdivided_coherence(&envelope, frame_secs, bpm);
    let steps = (BPM_SEARCH_RADIUS / BPM_SEARCH_STEP).round() as i32;
    let scored = (-steps..=steps)
        .map(|step| f64::from(raw.round()) + f64::from(step) * BPM_SEARCH_STEP)
        .filter(|&bpm| bpm >= f64::from(bpm_min) && bpm <= f64::from(bpm_max))
        .map(|bpm| (bpm, coherence(bpm)))
        .collect::<Vec<_>>();
    let clear_winner = |candidates: &[(f64, f64)]| {
        let &(bpm, score) = candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
        let rival = candidates
            .iter()
            .filter(|(other, _)| (other - bpm).abs() >= RIVAL_DISTANCE)
            .map(|&(_, score)| score)
            .fold(0.0, f64::max);
        (score >= MIN_BEAT_COHERENCE && score >= MIN_COHERENCE_LEAD * rival).then_some((bpm, score))
    };
    let Some((bpm, score)) = clear_winner(&scored) else {
        // A rhythm too loose for a sharp peak can still clearly favour one
        // whole or half BPM over its neighbours.
        let grid_only = scored
            .iter()
            .filter(|(bpm, _)| (bpm * 2.0 - (bpm * 2.0).round()).abs() < 1e-6)
            .map(|&(bpm, score)| ((bpm * 2.0).round() / 2.0, score))
            .collect::<Vec<_>>();
        return clear_winner(&grid_only).map(|(grid, score)| TempoPeak {
            bpm: grid,
            coherence: score,
            grid,
            grid_coherence: score,
        });
    };
    let grid = (bpm * 2.0).round() / 2.0;
    Some(TempoPeak {
        bpm,
        coherence: score,
        grid,
        grid_coherence: coherence(grid),
    })
}

/// `beat_coherence` averaged over the beat and its `SUBDIVISIONS`.
fn subdivided_coherence(envelope: &[f32], frame_secs: f64, bpm: f64) -> f64 {
    SUBDIVISIONS
        .iter()
        .map(|m| beat_coherence(envelope, frame_secs, bpm * m))
        .sum::<f64>()
        / SUBDIVISIONS.len() as f64
}

/// Rise in signal energy per `ONSET_HOP` frame (negative changes dropped):
/// peaks where notes and drums start.
fn onset_envelope(samples: &[f32]) -> Vec<f32> {
    let energy = samples
        .chunks(ONSET_HOP)
        .map(|frame| frame.iter().map(|x| x * x).sum::<f32>())
        .collect::<Vec<_>>();
    std::iter::once(0.0)
        .chain(energy.windows(2).map(|pair| (pair[1] - pair[0]).max(0.0)))
        .collect()
}

/// The part of the track the first beat is placed from: the grid has to fit
/// where it starts, and a tempo that wanders a little moves the phase
/// measured over the whole track away from the start.
const FIRST_BEAT_WINDOW_SECS: usize = 60;

/// Time of the first beat at `bpm`, within the first beat period: the phase
/// the onsets share on the beat picks which beat it is, and the phase on the
/// 2× and then 4× subdivisions, whose onsets are sharper and more frequent,
/// then places it more precisely. `None` for a silent start.
pub(super) fn first_beat_secs(samples: &[f32], sample_rate: u32, bpm: f64) -> Option<f64> {
    let window = &samples[..samples
        .len()
        .min(sample_rate as usize * FIRST_BEAT_WINDOW_SECS)];
    let envelope = onset_envelope(window);
    let frame_secs = ONSET_HOP as f64 / f64::from(sample_rate);
    let period = 60.0 / bpm;
    let mut first = None::<f64>;
    for multiple in [1.0, 2.0, 4.0] {
        let (re, im, total) = beat_phasor(&envelope, frame_secs, bpm * multiple);
        if total <= 0.0 {
            return None;
        }
        let step = period / multiple;
        let offset = im.atan2(re).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * step;
        // The subdivision onset nearest the beat found so far.
        first = Some(first.map_or(offset, |t| offset + ((t - offset) / step).round() * step));
    }
    first.map(|t| t.rem_euclid(period))
}

/// How consistently the onsets fall on the same beat phase at `bpm`, over
/// the whole track: 1.0 when every onset lands on the grid, near 0 when the
/// grid drifts through them.
fn beat_coherence(envelope: &[f32], frame_secs: f64, bpm: f64) -> f64 {
    let (re, im, total) = beat_phasor(envelope, frame_secs, bpm);
    if total <= 0.0 {
        return 0.0;
    }
    (re * re + im * im).sqrt() / total
}

/// The onset envelope summed as vectors whose angle is each frame's beat
/// phase at `bpm`, and the plain sum of its weights.
fn beat_phasor(envelope: &[f32], frame_secs: f64, bpm: f64) -> (f64, f64, f64) {
    // The beat phase advances by a fixed angle per frame: rotate a unit
    // vector instead of evaluating sin/cos for every frame.
    let (step_sin, step_cos) = (std::f64::consts::TAU * frame_secs * bpm / 60.0).sin_cos();
    let (mut cos, mut sin) = (1.0f64, 0.0f64);
    let (mut re, mut im, mut total) = (0.0f64, 0.0f64, 0.0f64);
    for &weight in envelope {
        let weight = f64::from(weight);
        re += weight * cos;
        im += weight * sin;
        total += weight;
        (cos, sin) = (
            cos * step_cos - sin * step_sin,
            sin * step_cos + cos * step_sin,
        );
    }
    (re, im, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_from_setting_defaults_to_stratum() {
        assert_eq!(AnalysisEngine::from_setting(""), AnalysisEngine::Stratum);
        assert_eq!(
            AnalysisEngine::from_setting("stratum"),
            AnalysisEngine::Stratum
        );
        assert_eq!(
            AnalysisEngine::from_setting("unknown"),
            AnalysisEngine::Stratum
        );
    }

    #[test]
    fn engine_from_setting_essentia() {
        assert_eq!(
            AnalysisEngine::from_setting("essentia"),
            AnalysisEngine::Essentia
        );
        assert_eq!(
            AnalysisEngine::from_setting("Essentia"),
            AnalysisEngine::Essentia
        );
        assert_eq!(
            AnalysisEngine::from_setting("  ESSENTIA  "),
            AnalysisEngine::Essentia
        );
    }

    #[test]
    fn stratum_empty_samples_returns_none() {
        let result = detect_bpm_key_stratum(&[], 44100, 70, 180).unwrap();
        assert!(result.bpm.is_none());
        assert!(result.key.is_none());
        assert!(result.first_beat_ms.is_none());
    }

    #[test]
    fn stratum_zero_sample_rate_returns_none() {
        let result = detect_bpm_key_stratum(&[0.1, 0.2, 0.3], 0, 70, 180).unwrap();
        assert!(result.bpm.is_none());
        assert!(result.key.is_none());
        assert!(result.first_beat_ms.is_none());
    }

    #[test]
    fn stratum_sine_produces_result() {
        // Generate a 10-second 440Hz sine wave at 44100Hz — should detect key of A
        let sample_rate = 44100u32;
        let duration_secs = 10.0f32;
        let freq = 440.0f32;
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin() * 0.8
            })
            .collect();

        let result = detect_bpm_key_stratum(&samples, sample_rate, 70, 180).unwrap();
        // A pure sine won't have clear BPM, but key detection should produce something
        assert!(result.key.is_some(), "expected key detection on sine wave");
    }

    /// Two minutes of kick-like clicks at `bpm`.
    fn click_track(bpm: f64, sample_rate: u32) -> Vec<f32> {
        click_track_secs(bpm, sample_rate, 120)
    }

    /// `secs` seconds of kick-like clicks at `bpm`.
    fn click_track_secs(bpm: f64, sample_rate: u32, secs: usize) -> Vec<f32> {
        let mut samples = vec![0f32; sample_rate as usize * secs];
        let interval = 60.0 / bpm * f64::from(sample_rate);
        let mut beat = 0.0;
        while (beat * interval) as usize + 2000 < samples.len() {
            let start = (beat * interval) as usize;
            for j in 0..2000 {
                let t = j as f32 / sample_rate as f32;
                samples[start + j] +=
                    (-(j as f32) / 300.0).exp() * (std::f32::consts::TAU * 60.0 * t).sin();
            }
            beat += 1.0;
        }
        samples
    }

    #[test]
    fn refine_bpm_keeps_a_half_bpm() {
        let samples = click_track(127.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 127.74, 70, 180), 127.5);
        // The slow half of a drum & bass tempo (175).
        let samples = click_track(87.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 87.37, 70, 180), 87.5);
    }

    #[test]
    fn refine_bpm_corrects_an_estimate_up_to_two_bpm_off() {
        let samples = click_track(128.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 128.2, 70, 180), 128.0);
        assert_eq!(refine_bpm(&samples, 44100, 129.8, 70, 180), 128.0);
    }

    #[test]
    fn refine_bpm_rounds_when_no_tempo_stands_out() {
        // Noise has no beat: every candidate scores alike.
        let mut seed = 1u32;
        let samples = (0..44100 * 60)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect::<Vec<_>>();
        assert_eq!(refine_bpm(&samples, 44100, 127.6, 70, 180), 128.0);
    }

    #[test]
    fn refine_bpm_stays_inside_the_analysis_range() {
        let samples = click_track(181.0, 44100);
        assert!(refine_bpm(&samples, 44100, 179.6, 70, 180) <= 180.0);
    }

    #[test]
    fn refine_bpm_finds_an_off_grid_tempo() {
        // Tracks as long as real ones: the closer a tempo is to a whole or
        // half BPM, the longer the track must be for the difference to show.
        let samples = click_track_secs(174.79, 44100, 300);
        let bpm = refine_bpm(&samples, 44100, 175.3, 70, 180);
        assert!((bpm - 174.79).abs() <= 0.02, "{bpm}");
        let samples = click_track_secs(86.88, 44100, 300);
        let bpm = refine_bpm(&samples, 44100, 87.4, 70, 180);
        assert!((bpm - 86.88).abs() <= 0.02, "{bpm}");
    }

    #[test]
    fn first_beat_lands_on_the_first_click() {
        // Clicks every 0.5 s, starting 0.2 s in.
        let mut samples = vec![0f32; 8820];
        samples.extend(click_track(120.0, 44100));
        let first = first_beat_secs(&samples, 44100, 120.0).unwrap();
        assert!((first - 0.2).abs() <= 0.01, "{first}");
    }

    #[test]
    fn refine_bpm_corrects_a_tempo_two_thirds_off() {
        // stratum answering 83 for a 125 track.
        let samples = click_track(125.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 83.2, 70, 180), 125.0);
    }

    #[test]
    fn refine_bpm_keeps_the_octave_of_the_estimate() {
        let samples = click_track(87.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 87.37, 70, 180), 87.5);
        assert_eq!(refine_bpm(&samples, 44100, 175.2, 70, 180), 175.0);
    }
}
