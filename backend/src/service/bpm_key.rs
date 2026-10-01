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

    let config = stratum_dsp::AnalysisConfig {
        min_bpm: bpm_min as f32,
        max_bpm: bpm_max as f32,
        ..Default::default()
    };

    let result = stratum_dsp::analyze_audio(samples, sample_rate, config)
        .map_err(|e| BackendError::Internal(format!("stratum-dsp analysis failed: {e}")))?;

    let bpm =
        (result.bpm > 0.0).then(|| refine_bpm(samples, sample_rate, result.bpm, bpm_min, bpm_max));

    let key = (!result.key.name().is_empty()).then(|| result.key.name());

    let first_beat_ms = result
        .beat_grid
        .beats
        .first()
        .map(|&t| (t * 1000.0).round() as u32);

    Ok(BpmKeyResult {
        bpm,
        key,
        first_beat_ms,
    })
}

/// Hop between onset-envelope frames, in samples.
const ONSET_HOP: usize = 512;
/// How far around stratum's estimate to look: its typical error is ±2 BPM.
const BPM_SEARCH_RADIUS: f32 = 2.0;
/// A candidate's beat coherence must reach this, and beat the runner-up by
/// `MIN_COHERENCE_LEAD`, to be trusted over plain rounding.
const MIN_BEAT_COHERENCE: f64 = 0.06;
const MIN_COHERENCE_LEAD: f64 = 1.5;

/// Turn stratum's raw estimate into the BPM stored for the track.
///
/// stratum searches whole-BPM steps and is typically off by up to ±2, so its
/// decimals mean nothing on their own. Instead, every whole and half BPM
/// within ±2 of it is tried against the whole track: a tempo even 0.5 BPM
/// off drifts more than a beat over a few minutes, so only the right one
/// keeps the onsets in phase. Half values matter most for slow tracks whose
/// double is the real tempo (86.5 → 173, 87.5 → 175). When no candidate
/// clearly wins (ambiguous rhythm, tempo changes), the estimate is rounded
/// to a whole BPM as before.
fn refine_bpm(samples: &[f32], sample_rate: u32, raw: f32, bpm_min: u32, bpm_max: u32) -> f64 {
    let rounded = f64::from(raw.round());
    let envelope = onset_envelope(samples);
    let frame_secs = ONSET_HOP as f64 / f64::from(sample_rate);
    let steps = (BPM_SEARCH_RADIUS * 2.0) as i32;
    let mut scored = (-steps..=steps)
        .map(|step| rounded + f64::from(step) * 0.5)
        .filter(|&bpm| bpm >= f64::from(bpm_min) && bpm <= f64::from(bpm_max))
        .map(|bpm| (bpm, beat_coherence(&envelope, frame_secs, bpm)))
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    match scored.as_slice() {
        [(best, score), (_, runner_up), ..]
            if *score >= MIN_BEAT_COHERENCE && *score >= MIN_COHERENCE_LEAD * runner_up =>
        {
            *best
        }
        _ => rounded,
    }
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

/// How consistently the onsets fall on the same beat phase at `bpm`, over
/// the whole track: 1.0 when every onset lands on the grid, near 0 when the
/// grid drifts through them.
fn beat_coherence(envelope: &[f32], frame_secs: f64, bpm: f64) -> f64 {
    let beats_per_frame = frame_secs * bpm / 60.0;
    let (mut re, mut im, mut total) = (0.0f64, 0.0f64, 0.0f64);
    for (i, &weight) in envelope.iter().enumerate() {
        let phase = std::f64::consts::TAU * (i as f64 * beats_per_frame);
        let weight = f64::from(weight);
        re += weight * phase.cos();
        im += weight * phase.sin();
        total += weight;
    }
    if total <= 0.0 {
        return 0.0;
    }
    (re * re + im * im).sqrt() / total
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
        let mut samples = vec![0f32; sample_rate as usize * 120];
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
}
