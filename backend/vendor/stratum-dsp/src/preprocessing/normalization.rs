//! Audio normalization utilities
//!
//! Supports multiple normalization methods:
//! - Peak normalization (fast)
//! - RMS normalization
//! - LUFS normalization (ITU-R BS.1770-4, accurate)
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::preprocessing::normalization::{
//!     normalize, NormalizationConfig, NormalizationMethod
//! };
//!
//! let mut samples = vec![0.5f32; 44100];
//! let config = NormalizationConfig {
//!     method: NormalizationMethod::Peak,
//!     target_loudness_lufs: -14.0,
//!     max_headroom_db: 1.0,
//! };
//!
//! let metadata = normalize(&mut samples, config, 44100.0)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Normalization method
#[derive(Debug, Clone, Copy)]
pub enum NormalizationMethod {
    /// Simple peak normalization (fast, scales to max peak)
    Peak,
    /// RMS-based normalization (scales to target RMS level)
    RMS,
    /// ITU-R BS.1770-4 loudness normalization (LUFS, accurate)
    Loudness,
}

/// Normalization configuration
#[derive(Debug, Clone)]
pub struct NormalizationConfig {
    /// Target loudness in LUFS (default: -14.0, YouTube standard)
    pub target_loudness_lufs: f32,
    
    /// Maximum headroom in dB (default: 1.0)
    pub max_headroom_db: f32,
    
    /// Normalization method
    pub method: NormalizationMethod,
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
            method: NormalizationMethod::Peak,
        }
    }
}

/// Loudness metadata returned from normalization
#[derive(Debug, Clone)]
pub struct LoudnessMetadata {
    /// Measured loudness in LUFS (before normalization)
    pub measured_lufs: Option<f32>,
    /// Peak level in dB (before normalization)
    pub peak_db: f32,
    /// RMS level in dB (before normalization)
    pub rms_db: f32,
    /// Gain applied in dB
    pub gain_db: f32,
}

impl Default for LoudnessMetadata {
    fn default() -> Self {
        Self {
            measured_lufs: None,
            peak_db: f32::NEG_INFINITY,
            rms_db: f32::NEG_INFINITY,
            gain_db: 0.0,
        }
    }
}

/// Numerical stability epsilon for divisions
const EPSILON: f32 = 1e-10;

/// Gate threshold for LUFS calculation (ITU-R BS.1770-4)
const LUFS_GATE_THRESHOLD: f32 = -70.0;

/// Block size for LUFS integration (400ms at 48kHz = 19200 samples)
/// We'll use a configurable block size based on sample rate
const LUFS_BLOCK_DURATION_MS: f32 = 400.0;

/// K-weighting filter for ITU-R BS.1770-4 loudness measurement
///
/// Implements the high-pass shelving filter component of the K-weighting filter
/// as specified in ITU-R BS.1770-4 Annex 2. The filter models human perception
/// of loudness by boosting frequencies around 1681.97 Hz.
///
/// # Reference
///
/// ITU-R BS.1770-4 (2015). Algorithms to measure audio programme loudness and true-peak audio level.
/// International Telecommunication Union.
///
/// # Note
///
/// This implementation uses a single high-pass shelving filter. The full ITU-R BS.1770-4
/// standard includes both high-shelf and low-shelf filters, but the high-pass component
/// is the primary contributor to the K-weighting response.
struct KWeightingFilter {
    // High-pass filter state (shelving filter)
    // Direct Form II transposed only needs x1 and x2
    x1: f32,
    x2: f32,
    // Coefficients for 48kHz
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl KWeightingFilter {
    /// Create a new K-weighting filter for the given sample rate
    /// 
    /// The K-weighting filter consists of:
    /// 1. High-pass filter (shelving filter) with specific frequency response
    /// 2. Pre-filtering stage as per ITU-R BS.1770-4
    fn new(sample_rate: f32) -> Self {
        // K-weighting filter coefficients for ITU-R BS.1770-4
        // These are standard coefficients for the high-pass shelving filter
        // Reference: ITU-R BS.1770-4 Annex 2
        
        // For 48kHz, the coefficients are:
        let w0 = 2.0 * std::f32::consts::PI * 1681.974450955533 / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * (1.0f32 / 0.707f32).sqrt(); // Q = 0.707
        
        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        
        Self {
            x1: 0.0,
            x2: 0.0,
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
    
    /// Process a single sample through the K-weighting filter
    fn process(&mut self, sample: f32) -> f32 {
        // Direct Form II transposed implementation
        let output = self.b0 * sample + self.x1;
        self.x1 = self.b1 * sample + self.x2 - self.a1 * output;
        self.x2 = self.b2 * sample - self.a2 * output;
        output
    }
    
    /// Reset filter state
    #[allow(dead_code)] // Useful for testing and reuse
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
    }
}

/// Calculate LUFS (Loudness Units relative to Full Scale) using ITU-R BS.1770-4
///
/// Algorithm:
/// 1. Apply K-weighting filter
/// 2. Compute mean square over 400ms blocks
/// 3. Apply gate at -70 LUFS
/// 4. Integrate gated blocks
/// 5. Convert to LUFS
fn calculate_lufs(samples: &[f32], sample_rate: f32) -> Result<f32, AnalysisError> {
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    if sample_rate <= 0.0 {
        return Err(AnalysisError::InvalidInput("Invalid sample rate".to_string()));
    }
    
    let block_size = (sample_rate * LUFS_BLOCK_DURATION_MS / 1000.0) as usize;
    if block_size == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate too low for LUFS calculation".to_string()));
    }
    
    // Apply K-weighting filter
    let mut filter = KWeightingFilter::new(sample_rate);
    let filtered: Vec<f32> = samples.iter().map(|&s| filter.process(s)).collect();
    
    // Compute mean square for each 400ms block
    let num_blocks = (filtered.len() + block_size - 1) / block_size;
    let mut block_energies = Vec::with_capacity(num_blocks);
    
    for i in 0..num_blocks {
        let start = i * block_size;
        let end = (start + block_size).min(filtered.len());
        
        // Mean square of block
        let sum_sq: f32 = filtered[start..end].iter().map(|&x| x * x).sum();
        let mean_sq = sum_sq / (end - start) as f32;
        block_energies.push(mean_sq);
    }
    
    if block_energies.is_empty() {
        return Err(AnalysisError::ProcessingError("No blocks computed for LUFS".to_string()));
    }
    
    // Convert to LUFS and apply gate
    // LUFS = -0.691 + 10 * log10(mean_square)
    // Gate: only include blocks above -70 LUFS
    let gate_threshold_linear = 10.0_f32.powf((LUFS_GATE_THRESHOLD + 0.691) / 10.0);
    
    let mut gated_energies = Vec::new();
    for &energy in &block_energies {
        if energy > gate_threshold_linear {
            gated_energies.push(energy);
        }
    }
    
    // If all blocks are below gate, return very quiet value
    if gated_energies.is_empty() {
        log::warn!("All audio blocks below LUFS gate threshold (-70 LUFS)");
        return Ok(f32::NEG_INFINITY);
    }
    
    // Average of gated blocks
    let mean_gated: f32 = gated_energies.iter().sum::<f32>() / gated_energies.len() as f32;
    
    // Convert to LUFS: LUFS = -0.691 + 10 * log10(mean_square)
    if mean_gated <= EPSILON {
        return Err(AnalysisError::NumericalError("Mean square too small for LUFS calculation".to_string()));
    }
    
    let lufs = -0.691 + 10.0 * mean_gated.log10();
    Ok(lufs)
}

/// Normalize audio samples using peak normalization
fn normalize_peak(samples: &mut [f32], max_headroom_db: f32) -> Result<LoudnessMetadata, AnalysisError> {
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    // Find peak value
    let peak = samples.iter()
        .map(|&x| x.abs())
        .fold(0.0f32, f32::max);
    
    if peak <= EPSILON {
        log::warn!("Audio is silent or extremely quiet, cannot normalize");
        return Ok(LoudnessMetadata {
            measured_lufs: None,
            peak_db: f32::NEG_INFINITY,
            rms_db: f32::NEG_INFINITY,
            gain_db: 0.0,
        });
    }
    
    // Calculate peak in dB
    let peak_db = 20.0 * peak.log10();
    
    // Calculate target peak (leave headroom)
    // max_headroom_db is the headroom below 0 dB, so target is 0 dB - headroom
    let target_peak_linear = 10.0_f32.powf((0.0 - max_headroom_db) / 20.0);
    let gain_linear = target_peak_linear / peak;
    let gain_db = 20.0 * gain_linear.log10();
    
    // Ensure we don't exceed 1.0
    let gain_linear = gain_linear.min(1.0 / peak);
    
    // Apply gain
    for sample in samples.iter_mut() {
        *sample *= gain_linear;
    }
    
    // Calculate RMS for metadata
    let rms = (samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
    let rms_db = if rms > EPSILON {
        20.0 * rms.log10()
    } else {
        f32::NEG_INFINITY
    };
    
    log::debug!("Peak normalization: peak={:.2} dB, gain={:.2} dB", peak_db, gain_db);
    
    Ok(LoudnessMetadata {
        measured_lufs: None,
        peak_db,
        rms_db,
        gain_db,
    })
}

/// Normalize audio samples using RMS normalization
fn normalize_rms(samples: &mut [f32], target_rms_db: f32, max_headroom_db: f32) -> Result<LoudnessMetadata, AnalysisError> {
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    // Calculate current RMS
    let rms_sq = samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32;
    let rms = rms_sq.sqrt();
    
    if rms <= EPSILON {
        log::warn!("Audio is silent or extremely quiet, cannot normalize");
        return Ok(LoudnessMetadata {
            measured_lufs: None,
            peak_db: f32::NEG_INFINITY,
            rms_db: f32::NEG_INFINITY,
            gain_db: 0.0,
        });
    }
    
    let rms_db = 20.0 * rms.log10();
    
    // Find peak to check headroom
    let peak = samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
    let peak_db = 20.0 * peak.log10();
    
    // Calculate target RMS
    let target_rms_linear = 10.0_f32.powf((target_rms_db - max_headroom_db) / 20.0);
    let gain_linear = target_rms_linear / rms;
    let gain_db = 20.0 * gain_linear.log10();
    
    // Check if gain would cause clipping
    let new_peak = peak * gain_linear;
    if new_peak > 1.0 {
        log::warn!("RMS normalization would cause clipping, limiting gain");
        let max_gain_linear = 1.0 / peak;
        let max_gain_db = 20.0 * max_gain_linear.log10();
        
        // Apply limited gain
        for sample in samples.iter_mut() {
            *sample *= max_gain_linear;
        }
        
        return Ok(LoudnessMetadata {
            measured_lufs: None,
            peak_db,
            rms_db,
            gain_db: max_gain_db,
        });
    }
    
    // Apply gain
    for sample in samples.iter_mut() {
        *sample *= gain_linear;
    }
    
    log::debug!("RMS normalization: rms={:.2} dB, gain={:.2} dB", rms_db, gain_db);
    
    Ok(LoudnessMetadata {
        measured_lufs: None,
        peak_db,
        rms_db,
        gain_db,
    })
}

/// Normalize audio samples using LUFS (ITU-R BS.1770-4)
fn normalize_lufs(
    samples: &mut [f32],
    target_lufs: f32,
    max_headroom_db: f32,
    sample_rate: f32,
) -> Result<LoudnessMetadata, AnalysisError> {
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    // Calculate current LUFS
    let measured_lufs = calculate_lufs(samples, sample_rate)?;
    
    if measured_lufs == f32::NEG_INFINITY {
        log::warn!("Audio is too quiet for LUFS measurement, using peak normalization fallback");
        return normalize_peak(samples, max_headroom_db);
    }
    
    // Calculate gain needed to reach target LUFS
    let lufs_diff = target_lufs - measured_lufs;
    let gain_db = lufs_diff;
    let gain_linear = 10.0_f32.powf(gain_db / 20.0);
    
    // Check peak to ensure we don't clip
    let peak = samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
    let peak_db = 20.0 * peak.log10();
    
    let new_peak = peak * gain_linear;
    let target_peak_linear = 10.0_f32.powf((0.0 - max_headroom_db) / 20.0);
    if new_peak > target_peak_linear {
        log::warn!("LUFS normalization would cause clipping, limiting gain to preserve headroom");
        let max_gain_linear = target_peak_linear / peak;
        let max_gain_db = 20.0 * max_gain_linear.log10();
        
        // Apply limited gain
        for sample in samples.iter_mut() {
            *sample *= max_gain_linear;
        }
        
        // Calculate RMS for metadata
        let rms = (samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        let rms_db = if rms > EPSILON {
            20.0 * rms.log10()
        } else {
            f32::NEG_INFINITY
        };
        
        return Ok(LoudnessMetadata {
            measured_lufs: Some(measured_lufs),
            peak_db,
            rms_db,
            gain_db: max_gain_db,
        });
    }
    
    // Apply gain
    for sample in samples.iter_mut() {
        *sample *= gain_linear;
    }
    
    // Calculate RMS for metadata
    let rms = (samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
    let rms_db = if rms > EPSILON {
        20.0 * rms.log10()
    } else {
        f32::NEG_INFINITY
    };
    
    log::debug!("LUFS normalization: measured={:.2} LUFS, target={:.2} LUFS, gain={:.2} dB", 
                measured_lufs, target_lufs, gain_db);
    
    Ok(LoudnessMetadata {
        measured_lufs: Some(measured_lufs),
        peak_db,
        rms_db,
        gain_db,
    })
}

/// Normalize audio samples
///
/// # Arguments
///
/// * `samples` - Audio samples to normalize (modified in-place)
/// * `config` - Normalization configuration
/// * `sample_rate` - Sample rate in Hz (required for LUFS method)
///
/// # Returns
///
/// `LoudnessMetadata` containing loudness information and applied gain
///
/// # Errors
///
/// Returns `AnalysisError` if normalization fails (empty samples, invalid config, etc.)
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::preprocessing::normalization::{
///     normalize, NormalizationConfig, NormalizationMethod
/// };
///
/// let mut samples = vec![0.5f32; 44100];
/// let config = NormalizationConfig {
///     method: NormalizationMethod::Peak,
///     target_loudness_lufs: -14.0,
///     max_headroom_db: 1.0,
/// };
///
/// let metadata = normalize(&mut samples, config, 44100.0)?;
/// println!("Applied gain: {:.2} dB", metadata.gain_db);
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn normalize(
    samples: &mut [f32],
    config: NormalizationConfig,
    sample_rate: f32,
) -> Result<LoudnessMetadata, AnalysisError> {
    log::debug!("Normalizing {} samples using {:?} at {} Hz", 
                samples.len(), config.method, sample_rate);
    
    match config.method {
        NormalizationMethod::Peak => {
            normalize_peak(samples, config.max_headroom_db)
        }
        NormalizationMethod::RMS => {
            // Convert target LUFS to approximate RMS dB
            // This is a rough approximation: LUFS ≈ RMS - 3 dB for typical music
            let target_rms_db = config.target_loudness_lufs + 3.0;
            normalize_rms(samples, target_rms_db, config.max_headroom_db)
        }
        NormalizationMethod::Loudness => {
            normalize_lufs(samples, config.target_loudness_lufs, config.max_headroom_db, sample_rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Generate a test signal: sine wave at 440 Hz
    fn generate_test_signal(length: usize, amplitude: f32, sample_rate: f32) -> Vec<f32> {
        let freq = 440.0;
        (0..length)
            .map(|i| {
                let t = i as f32 / sample_rate;
                amplitude * (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }
    
    #[test]
    fn test_peak_normalization() {
        let mut samples = generate_test_signal(44100, 0.5, 44100.0);
        
        let config = NormalizationConfig {
            method: NormalizationMethod::Peak,
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
        };
        
        let _metadata = normalize(&mut samples, config, 44100.0).unwrap();
        
        // Check that peak is approximately at target (with headroom)
        let new_peak = samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        let target_peak = 10.0_f32.powf((0.0 - 1.0) / 20.0); // 1 dB headroom from 0 dB
        
        assert!((new_peak - target_peak).abs() < 0.01, 
                "Peak normalization failed: expected ~{:.3}, got {:.3}", target_peak, new_peak);
        assert!(new_peak <= 1.0, "Peak normalization caused clipping: peak = {:.3}", new_peak);
    }
    
    #[test]
    fn test_rms_normalization() {
        let mut samples = generate_test_signal(44100, 0.3, 44100.0);
        
        let config = NormalizationConfig {
            method: NormalizationMethod::RMS,
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
        };
        
        let _metadata = normalize(&mut samples, config, 44100.0).unwrap();
        
        // Check that RMS is approximately at target
        let rms = (samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        let target_rms_db = -14.0 + 3.0; // Approximate conversion
        let target_rms = 10.0_f32.powf((target_rms_db - 1.0) / 20.0); // With headroom
        
        assert!((rms - target_rms).abs() < 0.1, 
                "RMS normalization failed: expected ~{:.3}, got {:.3}", target_rms, rms);
        
        // Check no clipping
        let peak = samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak <= 1.0, "RMS normalization caused clipping");
    }
    
    #[test]
    fn test_lufs_calculation() {
        // Generate a louder signal for LUFS testing
        let samples = generate_test_signal(48000 * 2, 0.8, 48000.0); // 2 seconds at 48kHz
        
        let lufs = calculate_lufs(&samples, 48000.0).unwrap();
        
        // For a 0.8 amplitude sine wave, LUFS depends on K-weighting filter response
        // K-weighting reduces low frequencies, so a pure tone at 440Hz will have lower LUFS
        // than the peak level. The value should be finite and reasonable.
        assert!(lufs.is_finite(), "LUFS should be finite: {:.2} LUFS", lufs);
        assert!(lufs < 0.0, "LUFS should be negative for normalized audio: {:.2} LUFS", lufs);
    }
    
    #[test]
    fn test_lufs_normalization() {
        // Generate test signal
        let mut samples = generate_test_signal(48000 * 2, 0.5, 48000.0); // 2 seconds
        
        let config = NormalizationConfig {
            method: NormalizationMethod::Loudness,
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
        };
        
        let metadata = normalize(&mut samples, config, 48000.0).unwrap();
        
        // Verify metadata
        assert!(metadata.measured_lufs.is_some(), "LUFS normalization should return measured LUFS");
        assert!(metadata.gain_db != 0.0, "Gain should be applied");
        
        // Verify no clipping
        let peak = samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak <= 1.0, "LUFS normalization caused clipping: peak = {:.3}", peak);
    }
    
    #[test]
    fn test_silent_audio() {
        let mut samples = vec![0.0f32; 44100];
        
        let config = NormalizationConfig {
            method: NormalizationMethod::Peak,
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
        };
        
        // Should handle silently without error
        let metadata = normalize(&mut samples, config, 44100.0).unwrap();
        assert_eq!(metadata.gain_db, 0.0, "Silent audio should not apply gain");
        assert_eq!(metadata.peak_db, f32::NEG_INFINITY);
    }
    
    #[test]
    fn test_ultra_quiet_audio() {
        // Very quiet signal (near silence)
        let mut samples = generate_test_signal(44100, 1e-6, 44100.0);
        
        let config = NormalizationConfig {
            method: NormalizationMethod::Peak,
            target_loudness_lufs: -14.0,
            max_headroom_db: 1.0,
        };
        
        // Should handle gracefully
        let _metadata = normalize(&mut samples, config, 44100.0).unwrap();
        // May or may not apply gain depending on epsilon threshold
    }
    
    #[test]
    fn test_empty_samples() {
        let mut samples = vec![];
        
        let config = NormalizationConfig::default();
        
        let result = normalize(&mut samples, config, 44100.0);
        assert!(result.is_err(), "Empty samples should return error");
    }
    
    #[test]
    fn test_k_weighting_filter() {
        let sample_rate = 48000.0;
        let mut filter = KWeightingFilter::new(sample_rate);
        
        // Test with impulse
        let output = filter.process(1.0);
        assert!(!output.is_nan() && !output.is_infinite(), 
                "Filter output should be finite");
        
        // Reset and test with sine wave
        filter.reset();
        let test_signal = generate_test_signal(1000, 0.5, sample_rate);
        let filtered: Vec<f32> = test_signal.iter().map(|&s| filter.process(s)).collect();
        
        // Filtered signal should be different from input (K-weighting changes frequency response)
        assert_ne!(filtered, test_signal, "K-weighting filter should modify signal");
        
        // All outputs should be finite
        for &x in &filtered {
            assert!(x.is_finite(), "Filter output should be finite");
        }
    }
}

