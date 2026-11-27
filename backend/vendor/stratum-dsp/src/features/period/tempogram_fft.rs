//! FFT-based tempogram for BPM detection
//!
//! Applies FFT to the novelty curve and converts frequencies to BPM.
//! The frequency with highest FFT power indicates the true tempo.
//!
//! # Reference
//!
//! Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
//! *IEEE Transactions on Audio, Speech, and Language Processing*.
//!
//! # Algorithm
//!
//! 1. Apply FFT to novelty curve: `fft_output = FFT(novelty_curve)`
//! 2. Convert frequency bins to BPM: `BPM = Hz * 60`
//! 3. Filter within BPM range (40-240 BPM)
//! 4. Find BPM with highest FFT power
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::tempogram_fft::fft_tempogram;
//!
//! let novelty_curve = vec![0.0f32; 1000];
//! let tempogram = fft_tempogram(&novelty_curve, 44100, 512, 40.0, 240.0)?;
//! // tempogram is Vec<(f32, f32)> where each pair is (BPM, power)
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// BPM estimate with confidence from FFT tempogram
#[derive(Debug, Clone)]
pub struct FftTempogramResult {
    /// BPM estimate
    pub bpm: f32,
    
    /// Confidence score (0.0-1.0) based on peak prominence
    pub confidence: f32,
    
    /// FFT power at this BPM
    pub power: f32,
}

/// Compute FFT tempogram
///
/// Applies FFT to the novelty curve and converts frequencies to BPM.
/// The frequency with highest FFT power indicates the true tempo.
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
///
/// # Returns
///
/// Tempogram as vector of (BPM, power) pairs, sorted by power (highest first)
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Novelty curve is too short
/// - Invalid parameters (sample_rate=0, hop_size=0)
/// - BPM range is invalid
/// - FFT computation fails
pub fn fft_tempogram(
    novelty_curve: &[f32],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
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
    
    // Compute frame rate (frames per second)
    let frame_rate = sample_rate as f32 / hop_size as f32;
    
    log::debug!("Computing FFT tempogram: {} novelty values, frame_rate={:.2} Hz, BPM range=[{:.1}, {:.1}]",
                novelty_curve.len(), frame_rate, min_bpm, max_bpm);
    
    // Preconditioning:
    // - Remove DC (mean) so the 0 Hz component doesn't dominate
    // - Apply Hann window to reduce spectral leakage
    //
    // This is standard practice for frequency-domain periodicity analysis.
    let mean = novelty_curve.iter().copied().sum::<f32>() / novelty_curve.len() as f32;

    // Zero-pad to next power of 2 for efficient FFT
    let n = novelty_curve.len();
    let fft_size = n.next_power_of_two();
    
    // Prepare FFT input (zero-padded)
    let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(fft_size);
    for (i, &x) in novelty_curve.iter().enumerate() {
        // Hann window
        let w = if n > 1 {
            let t = 2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32;
            0.5 * (1.0 - t.cos())
        } else {
            1.0
        };
        fft_input.push(Complex::new((x - mean) * w, 0.0));
    }
    
    // Zero-pad to fft_size
    fft_input.resize(fft_size, Complex::new(0.0, 0.0));
    
    // Compute FFT
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    fft.process(&mut fft_input);
    
    // Compute power spectrum (magnitude squared)
    let power_spectrum: Vec<f32> = fft_input.iter()
        .map(|x| {
            let mag_sq = x.re * x.re + x.im * x.im;
            mag_sq
        })
        .collect();
    
    // Convert frequency bins to BPM
    let mut tempogram = Vec::new();
    
    // Frequency resolution: frame_rate / fft_size
    let freq_resolution = frame_rate / fft_size as f32;
    
    // Only consider frequencies up to Nyquist (fft_size / 2)
    let max_bin = fft_size / 2;
    
    for (bin_idx, &power) in power_spectrum.iter().take(max_bin + 1).enumerate() {
        // Convert bin index to frequency in Hz
        let freq_hz = bin_idx as f32 * freq_resolution;
        
        // Convert frequency to BPM: BPM = Hz * 60
        let bpm = freq_hz * 60.0;
        
        // Filter within BPM range
        if bpm >= min_bpm && bpm <= max_bpm {
            tempogram.push((bpm, power));
        }
    }
    
    // Sort by power (highest first)
    tempogram.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    log::debug!("FFT tempogram: {} BPM candidates, top BPM={:.1} (power={:.6})",
                tempogram.len(),
                tempogram.first().map(|(bpm, _)| *bpm).unwrap_or(0.0),
                tempogram.first().map(|(_, power)| *power).unwrap_or(0.0));
    
    Ok(tempogram)
}

/// Find best BPM estimate from FFT tempogram
///
/// Extracts the BPM with highest FFT power and computes confidence based on
/// peak prominence (ratio of top peak to second peak).
///
/// # Arguments
///
/// * `tempogram` - Tempogram from `fft_tempogram()`
///
/// # Returns
///
/// Best BPM estimate with confidence, or None if tempogram is empty
pub fn find_best_bpm_fft(
    tempogram: &[(f32, f32)],
) -> Option<FftTempogramResult> {
    if tempogram.is_empty() {
        return None;
    }
    
    let (best_bpm, best_power) = tempogram[0];
    
    // Compute confidence based on peak prominence
    // Compare top peak to second peak (if exists)
    let confidence = if tempogram.len() > 1 {
        let second_power = tempogram[1].1;
        if best_power > EPSILON {
            // Prominence-style confidence: (best - second) / best
            // - 1.0 when second is 0
            // - 0.0 when second equals best
            let prom = (best_power - second_power).max(0.0) / best_power;
            prom.clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        // Only one candidate, moderate confidence
        0.5
    };
    
    Some(FftTempogramResult {
        bpm: best_bpm,
        confidence,
        power: best_power,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fft_tempogram_periodic() {
        // Create periodic novelty curve (120 BPM = 2 beats per second)
        // At 44.1kHz, 512 hop: frame_rate = 44100/512 ≈ 86.13 Hz
        // For 120 BPM: frequency = 2 Hz
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
        
        let tempogram = fft_tempogram(&novelty, sample_rate, hop_size, 100.0, 140.0).unwrap();
        
        // Should find 120 BPM (or close) as top candidate
        let best = find_best_bpm_fft(&tempogram).unwrap();
        assert!(best.bpm >= 115.0 && best.bpm <= 125.0,
                "Expected BPM around 120, got {:.1}", best.bpm);
    }
    
    #[test]
    fn test_fft_tempogram_empty() {
        let novelty = vec![];
        let result = fft_tempogram(&novelty, 44100, 512, 40.0, 240.0);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_fft_tempogram_invalid_params() {
        let novelty = vec![0.5f32; 100];
        
        // Invalid sample rate
        let result = fft_tempogram(&novelty, 0, 512, 40.0, 240.0);
        assert!(result.is_err());
        
        // Invalid hop size
        let result = fft_tempogram(&novelty, 44100, 0, 40.0, 240.0);
        assert!(result.is_err());
        
        // Invalid BPM range
        let result = fft_tempogram(&novelty, 44100, 512, 240.0, 40.0);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_find_best_bpm_fft() {
        let tempogram = vec![
            (120.0, 0.9),
            (60.0, 0.3),
            (180.0, 0.2),
        ];
        
        let result = find_best_bpm_fft(&tempogram).unwrap();
        
        assert_eq!(result.bpm, 120.0);
        assert_eq!(result.power, 0.9);
        // Confidence is prominence-style: (best - second) / best = (0.9 - 0.3) / 0.9 = 0.666...
        assert!((result.confidence - (2.0 / 3.0)).abs() < 1e-6);
    }
    
    #[test]
    fn test_find_best_bpm_fft_empty() {
        let tempogram = vec![];
        let result = find_best_bpm_fft(&tempogram);
        assert!(result.is_none());
    }
}

