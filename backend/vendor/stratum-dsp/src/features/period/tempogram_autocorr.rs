//! Autocorrelation-based tempogram for BPM detection
//!
//! Tests each BPM hypothesis by computing autocorrelation of the novelty curve
//! at the corresponding tempo lag. The BPM with highest autocorrelation strength
//! is the true tempo.
//!
//! # Reference
//!
//! Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
//! *IEEE Transactions on Audio, Speech, and Language Processing*.
//!
//! # Algorithm
//!
//! For each BPM candidate (40-240, 0.5 BPM resolution):
//! 1. Convert BPM to period in frames: `frames_per_beat = frame_rate / (BPM / 60.0)`
//! 2. Compute autocorrelation at this lag: `autocorr = sum(novelty[i] * novelty[i + lag])`
//! 3. Normalize by count: `strength = autocorr / count`
//! 4. Find BPM with highest strength
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::tempogram_autocorr::autocorrelation_tempogram;
//!
//! let novelty_curve = vec![0.0f32; 1000];
//! let tempogram = autocorrelation_tempogram(&novelty_curve, 44100, 512, 40.0, 240.0, 0.5)?;
//! // tempogram is Vec<(f32, f32)> where each pair is (BPM, strength)
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// BPM estimate with confidence from autocorrelation tempogram
#[derive(Debug, Clone)]
pub struct AutocorrTempogramResult {
    /// BPM estimate
    pub bpm: f32,
    
    /// Confidence score (0.0-1.0) based on peak prominence
    pub confidence: f32,
    
    /// Autocorrelation strength at this BPM
    pub strength: f32,
}

/// Compute autocorrelation tempogram
///
/// Tests each BPM hypothesis by computing autocorrelation of the novelty curve
/// at the corresponding tempo lag. The BPM with highest autocorrelation strength
/// indicates the true tempo.
///
/// # Reference
///
/// Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
/// *IEEE Transactions on Audio, Speech, and Language Processing*.
///
/// # Arguments
///
/// * `novelty_curve` - Novelty curve (one value per frame transition)
/// * `sample_rate` - Sample rate in Hz
/// * `hop_size` - Hop size used for STFT (samples per frame)
/// * `min_bpm` - Minimum BPM to consider (default: 40.0)
/// * `max_bpm` - Maximum BPM to consider (default: 240.0)
/// * `bpm_resolution` - BPM resolution for search (default: 0.5)
///
/// # Returns
///
/// Tempogram as vector of (BPM, strength) pairs, sorted by strength (highest first)
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Novelty curve is too short
/// - Invalid parameters (sample_rate=0, hop_size=0)
/// - BPM range is invalid
pub fn autocorrelation_tempogram(
    novelty_curve: &[f32],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
) -> Result<Vec<(f32, f32)>, AnalysisError> {
    if novelty_curve.is_empty() {
        return Err(AnalysisError::InvalidInput("Novelty curve is empty".to_string()));
    }
    
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
    }
    
    if hop_size == 0 {
        return Err(AnalysisError::InvalidInput("Hop size must be > 0".to_string()));
    }
    
    if min_bpm <= 0.0 || max_bpm <= min_bpm {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid BPM range: min={}, max={}", min_bpm, max_bpm)
        ));
    }
    
    if bpm_resolution <= 0.0 {
        return Err(AnalysisError::InvalidInput(
            format!("BPM resolution must be > 0, got {}", bpm_resolution)
        ));
    }
    
    // Compute frame rate (frames per second)
    let frame_rate = sample_rate as f32 / hop_size as f32;
    
    log::debug!("Computing autocorrelation tempogram: {} novelty values, frame_rate={:.2} Hz, BPM range=[{:.1}, {:.1}], resolution={:.1}",
                novelty_curve.len(), frame_rate, min_bpm, max_bpm, bpm_resolution);
    
    // For each BPM hypothesis, compute autocorrelation at tempo lag
    let mut tempogram = Vec::new();
    
    let mut bpm = min_bpm;
    while bpm <= max_bpm {
        // Convert BPM to period in frames
        // BPM = beats per minute = beats_per_second * 60
        // frames_per_beat = frame_rate / beats_per_second = frame_rate / (BPM / 60.0)
        let beats_per_second = bpm / 60.0;
        let frames_per_beat = frame_rate / beats_per_second;
        
        // Compute autocorrelation at this lag
        let mut autocorr_sum = 0.0;
        let mut autocorr_count = 0;
        
        let lag_frames = frames_per_beat as usize;
        
        // For each frame, compare with frame at tempo lag
        for i in 0..novelty_curve.len() {
            let j = i + lag_frames;
            if j < novelty_curve.len() {
                // Autocorrelation: compare frames at tempo interval
                autocorr_sum += novelty_curve[i] * novelty_curve[j];
                autocorr_count += 1;
            }
        }
        
        // Normalize by count to get average autocorrelation strength
        let strength = if autocorr_count > 0 {
            autocorr_sum / autocorr_count as f32
        } else {
            0.0
        };
        
        tempogram.push((bpm, strength));
        
        bpm += bpm_resolution;
    }
    
    // Sort by strength (highest first)
    tempogram.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    log::debug!("Autocorrelation tempogram: {} BPM candidates, top BPM={:.1} (strength={:.6})",
                tempogram.len(),
                tempogram.first().map(|(bpm, _)| *bpm).unwrap_or(0.0),
                tempogram.first().map(|(_, strength)| *strength).unwrap_or(0.0));
    
    Ok(tempogram)
}

/// Find best BPM estimate from autocorrelation tempogram
///
/// Extracts the BPM with highest autocorrelation strength and computes
/// confidence based on peak prominence (ratio of top peak to second peak).
///
/// # Arguments
///
/// * `tempogram` - Tempogram from `autocorrelation_tempogram()`
///
/// # Returns
///
/// Best BPM estimate with confidence, or None if tempogram is empty
pub fn find_best_bpm_autocorr(
    tempogram: &[(f32, f32)],
) -> Option<AutocorrTempogramResult> {
    if tempogram.is_empty() {
        return None;
    }
    
    let (best_bpm, best_strength) = tempogram[0];
    
    // Compute confidence based on peak prominence
    // Compare top peak to second peak (if exists)
    let confidence = if tempogram.len() > 1 {
        let second_strength = tempogram[1].1;
        if best_strength > EPSILON {
            // Prominence-style confidence: (best - second) / best
            // - 1.0 when second is 0
            // - 0.0 when second equals best
            let prom = (best_strength - second_strength).max(0.0) / best_strength;
            prom.clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        // Only one candidate, moderate confidence
        0.5
    };
    
    Some(AutocorrTempogramResult {
        bpm: best_bpm,
        confidence,
        strength: best_strength,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_autocorrelation_tempogram_periodic() {
        // Create periodic novelty curve (120 BPM = 2 beats per second)
        // At 44.1kHz, 512 hop: frame_rate = 44100/512 ≈ 86.13 Hz
        // For 120 BPM: frames_per_beat = 86.13 / 2 ≈ 43 frames
        let sample_rate = 44100;
        let hop_size = 512;
        let frame_rate = sample_rate as f32 / hop_size as f32;
        let beats_per_second = 120.0 / 60.0;
        let frames_per_beat = frame_rate / beats_per_second;
        
        // Create novelty curve with periodicity at frames_per_beat
        let period = frames_per_beat as usize;
        let mut novelty = vec![0.0f32; 500];
        for i in 0..novelty.len() {
            if i % period == 0 {
                novelty[i] = 1.0;
            }
        }
        
        let tempogram = autocorrelation_tempogram(&novelty, sample_rate, hop_size, 100.0, 140.0, 1.0).unwrap();
        
        // Should find 120 BPM (or close) as top candidate
        let best = find_best_bpm_autocorr(&tempogram).unwrap();
        assert!(best.bpm >= 115.0 && best.bpm <= 125.0,
                "Expected BPM around 120, got {:.1}", best.bpm);
    }
    
    #[test]
    fn test_autocorrelation_tempogram_empty() {
        let novelty = vec![];
        let result = autocorrelation_tempogram(&novelty, 44100, 512, 40.0, 240.0, 0.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_autocorrelation_tempogram_invalid_params() {
        let novelty = vec![0.5f32; 100];
        
        // Invalid sample rate
        let result = autocorrelation_tempogram(&novelty, 0, 512, 40.0, 240.0, 0.5);
        assert!(result.is_err());
        
        // Invalid hop size
        let result = autocorrelation_tempogram(&novelty, 44100, 0, 40.0, 240.0, 0.5);
        assert!(result.is_err());
        
        // Invalid BPM range
        let result = autocorrelation_tempogram(&novelty, 44100, 512, 240.0, 40.0, 0.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_find_best_bpm_autocorr() {
        let tempogram = vec![
            (120.0, 0.9),
            (60.0, 0.3),
            (180.0, 0.2),
        ];
        
        let result = find_best_bpm_autocorr(&tempogram).unwrap();
        
        assert_eq!(result.bpm, 120.0);
        assert_eq!(result.strength, 0.9);
        // Confidence is prominence-style: (best - second) / best = (0.9 - 0.3) / 0.9 = 0.666...
        assert!((result.confidence - (2.0 / 3.0)).abs() < 1e-6);
    }
    
    #[test]
    fn test_find_best_bpm_autocorr_empty() {
        let tempogram = vec![];
        let result = find_best_bpm_autocorr(&tempogram);
        assert!(result.is_none());
    }
}

