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

    let bpm = if result.bpm > 0.0 {
        Some(result.bpm.round() as f64)
    } else {
        None
    };

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
}
