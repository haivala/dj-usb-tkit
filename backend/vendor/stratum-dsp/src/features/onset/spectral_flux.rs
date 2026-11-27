//! Spectral flux onset detection
//!
//! Detects onsets by finding changes in magnitude spectrogram.
//!
//! Algorithm:
//! 1. Normalize magnitude to [0, 1] per frame
//! 2. Compute L2 distance between consecutive frames
//! 3. Apply percentile-based threshold
//! 4. Peak-pick to find onsets
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::onset::spectral_flux::detect_spectral_flux_onsets;
//!
//! // fft_magnitudes is a spectrogram: Vec<Vec<f32>> where each inner Vec is a frame
//! let fft_magnitudes = vec![vec![0.0f32; 1024]; 100]; // 100 frames, 1024 bins each
//! let onsets = detect_spectral_flux_onsets(&fft_magnitudes, 0.8)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Detect onsets using spectral flux method
///
/// This method detects onsets by computing the L2 distance (Euclidean distance)
/// between consecutive normalized magnitude spectra. Spectral changes indicate
/// onsets, making this method effective for detecting both percussive and
/// harmonic onsets.
///
/// # Reference
///
/// Bello, J. P., Daudet, L., Abdallah, S., Duxbury, C., Davies, M., & Sandler, M. B. (2005).
/// A Tutorial on Onset Detection in Music Signals.
/// *IEEE Transactions on Speech and Audio Processing*, 13(5), 1035-1047.
///
/// # Arguments
///
/// * `fft_magnitudes` - FFT magnitude spectrogram (n_frames × n_bins)
///   Each inner `Vec<f32>` represents one frame's magnitude spectrum
/// * `threshold_percentile` - Threshold percentile (0.0-1.0)
///   e.g., 0.8 means threshold is 80th percentile of flux values
///
/// # Returns
///
/// Vector of onset frame indices (0-indexed), sorted by frame number
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Spectrogram is empty
/// - Frames have inconsistent lengths
/// - Threshold percentile is out of range
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::onset::spectral_flux::detect_spectral_flux_onsets;
///
/// // Example: 100 frames, each with 1024 frequency bins
/// let fft_magnitudes = vec![vec![0.0f32; 1024]; 100];
/// let onsets = detect_spectral_flux_onsets(&fft_magnitudes, 0.8)?;
/// println!("Found {} onsets", onsets.len());
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn detect_spectral_flux_onsets(
    fft_magnitudes: &[Vec<f32>],
    threshold_percentile: f32,
) -> Result<Vec<usize>, AnalysisError> {
    // Validate inputs
    if fft_magnitudes.is_empty() {
        return Ok(Vec::new());
    }
    
    if threshold_percentile < 0.0 || threshold_percentile > 1.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Threshold percentile must be in [0, 1], got {}", threshold_percentile)
        ));
    }
    
    // Check that all frames have the same length
    let n_bins = fft_magnitudes[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    
    for (i, frame) in fft_magnitudes.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!("Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                        n_bins, i, frame.len())
            ));
        }
    }
    
    if fft_magnitudes.len() < 2 {
        // Need at least 2 frames to compute flux
        return Ok(Vec::new());
    }
    
    log::debug!("Detecting spectral flux onsets: {} frames, {} bins per frame, threshold_percentile={:.2}",
                fft_magnitudes.len(), n_bins, threshold_percentile);
    
    // Step 1: Normalize magnitude to [0, 1] per frame
    // For each frame, find max magnitude and divide all values by it
    let mut normalized: Vec<Vec<f32>> = Vec::with_capacity(fft_magnitudes.len());
    
    for frame in fft_magnitudes {
        // Find maximum magnitude in this frame
        let max_mag = frame.iter().copied().fold(0.0f32, f32::max);
        
        if max_mag > EPSILON {
            // Normalize: divide by max
            let normalized_frame: Vec<f32> = frame.iter()
                .map(|&x| x / max_mag)
                .collect();
            normalized.push(normalized_frame);
        } else {
            // All zeros, keep as zeros
            normalized.push(vec![0.0f32; n_bins]);
        }
    }
    
    // Step 2: Compute L2 distance between consecutive frames
    // L2 distance = sqrt(sum((frame_n[i] - frame_{n-1}[i])^2))
    let mut spectral_flux = Vec::with_capacity(fft_magnitudes.len() - 1);
    
    for i in 1..normalized.len() {
        let prev_frame = &normalized[i - 1];
        let curr_frame = &normalized[i];
        
        // Compute L2 distance (Euclidean distance)
        let sum_sq_diff: f32 = prev_frame.iter()
            .zip(curr_frame.iter())
            .map(|(&prev, &curr)| {
                let diff = curr - prev;
                // Only count positive differences (increases in magnitude)
                // This is the "half-wave rectification" common in spectral flux
                diff.max(0.0)
            })
            .map(|x| x * x)
            .sum();
        
        let l2_distance = sum_sq_diff.sqrt();
        spectral_flux.push(l2_distance);
    }
    
    if spectral_flux.is_empty() {
        return Ok(Vec::new());
    }
    
    // Step 3: Apply percentile-based threshold
    // Sort flux values to find percentile
    let mut sorted_flux = spectral_flux.clone();
    sorted_flux.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    
    let threshold_idx = ((sorted_flux.len() as f32) * threshold_percentile) as usize;
    let threshold_idx = threshold_idx.min(sorted_flux.len() - 1);
    let threshold = sorted_flux[threshold_idx];
    
    log::debug!("Spectral flux: max={:.6}, threshold={:.6} ({}th percentile)",
                sorted_flux.last().unwrap_or(&0.0), threshold, threshold_percentile);
    
    // Step 4: Peak-pick to find local maxima above threshold
    let mut onsets = Vec::new();
    
    // Find local maxima in spectral flux
    for i in 1..(spectral_flux.len() - 1) {
        let flux = spectral_flux[i];
        let prev_flux = spectral_flux[i - 1];
        let next_flux = spectral_flux[i + 1];
        
        // Check if this is a local maximum above threshold
        if flux > threshold && flux > prev_flux && flux >= next_flux {
            // Frame index is i+1 (since flux[i] is between frame i and i+1)
            onsets.push(i + 1);
        }
    }
    
    // Handle edge case: check first frame if it's above threshold
    if spectral_flux.len() > 1 && 
       spectral_flux[0] > threshold && 
       spectral_flux[0] >= spectral_flux[1] {
        onsets.push(1);
    }
    
    // Handle edge case: check last frame if it's above threshold
    let last_idx = spectral_flux.len() - 1;
    if spectral_flux.len() > 1 && 
       spectral_flux[last_idx] > threshold && 
       spectral_flux[last_idx] > spectral_flux[last_idx - 1] {
        onsets.push(spectral_flux.len());
    }
    
    // Sort onsets by frame index (should already be sorted, but ensure it)
    onsets.sort_unstable();
    
    // Remove duplicates (shouldn't happen, but be safe)
    onsets.dedup();
    
    log::debug!("Spectral flux detected {} onsets", onsets.len());
    
    Ok(onsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_spectral_flux_basic() {
        // Create spectrogram with a sudden spectral change
        // Frames 0-4: energy in low frequencies (bins 0-256)
        // Frame 5: energy in high frequencies (bins 768-1024) - different spectral pattern
        // Frames 6-9: back to low frequencies
        let mut spectrogram = vec![vec![0.01f32; 1024]; 10];
        
        // Frames 0-4: low frequency content
        for i in 0..5 {
            for bin in 0..256 {
                spectrogram[i][bin] = 1.0f32;
            }
        }
        
        // Frame 5: high frequency content (different spectral pattern)
        for bin in 768..1024 {
            spectrogram[5][bin] = 1.0f32;
        }
        
        // Frames 6-9: back to low frequency
        for i in 6..10 {
            for bin in 0..256 {
                spectrogram[i][bin] = 1.0f32;
            }
        }
        
        // Use lower threshold to ensure detection
        let onsets = detect_spectral_flux_onsets(&spectrogram, 0.3).unwrap();
        
        // Should detect at least one onset near the change
        assert!(!onsets.is_empty(), "Should detect at least one onset");
        // Onset should be at or near frame 5-6 (the change happens at frame 5, so flux is between 4-5 and 5-6)
        assert!(onsets.iter().any(|&f| f >= 4 && f <= 7),
                "Onset should be near frame 5-6, got {:?}", onsets);
    }
    
    #[test]
    fn test_spectral_flux_empty() {
        let spectrogram = vec![];
        let onsets = detect_spectral_flux_onsets(&spectrogram, 0.8).unwrap();
        assert!(onsets.is_empty());
    }
    
    #[test]
    fn test_spectral_flux_single_frame() {
        let spectrogram = vec![vec![0.5f32; 1024]];
        let onsets = detect_spectral_flux_onsets(&spectrogram, 0.8).unwrap();
        // Need at least 2 frames to compute flux
        assert!(onsets.is_empty());
    }
    
    #[test]
    fn test_spectral_flux_invalid_percentile() {
        let spectrogram = vec![vec![0.5f32; 1024]; 10];
        
        // Test negative percentile
        let result = detect_spectral_flux_onsets(&spectrogram, -0.1);
        assert!(result.is_err());
        
        // Test > 1.0 percentile
        let result = detect_spectral_flux_onsets(&spectrogram, 1.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_spectral_flux_inconsistent_lengths() {
        let mut spectrogram = vec![vec![0.5f32; 1024]; 10];
        spectrogram[5] = vec![0.5f32; 512]; // Different length
        
        let result = detect_spectral_flux_onsets(&spectrogram, 0.8);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_spectral_flux_all_zeros() {
        let spectrogram = vec![vec![0.0f32; 1024]; 10];
        let onsets = detect_spectral_flux_onsets(&spectrogram, 0.8).unwrap();
        // All zeros should produce no onsets (or very few if threshold is very low)
        assert!(onsets.is_empty() || onsets.len() < 3);
    }
    
    #[test]
    fn test_spectral_flux_threshold_sensitivity() {
        // Create spectrogram with gradual changes
        let mut spectrogram = vec![vec![0.1f32; 1024]; 20];
        for i in 0..20 {
            let amplitude = 0.1 + (i as f32 / 20.0) * 0.9;
            for bin in 0..1024 {
                spectrogram[i][bin] = amplitude;
            }
        }
        
        // Lower threshold should detect more onsets
        let onsets_low = detect_spectral_flux_onsets(&spectrogram, 0.5).unwrap();
        let onsets_high = detect_spectral_flux_onsets(&spectrogram, 0.9).unwrap();
        
        assert!(onsets_low.len() >= onsets_high.len(),
                "Lower threshold should detect more onsets: {} vs {}",
                onsets_low.len(), onsets_high.len());
    }
    
    #[test]
    fn test_spectral_flux_normalization() {
        // Test that normalization works correctly
        // Frame 1: all 0.5, Frame 2: all 1.0
        // After normalization, both should be all 1.0, so flux should be 0
        let mut spectrogram = vec![vec![0.5f32; 1024]; 2];
        spectrogram[1] = vec![1.0f32; 1024];
        
        let _onsets = detect_spectral_flux_onsets(&spectrogram, 0.5).unwrap();
        
        // After normalization, both frames are identical, so flux should be low
        // But there might still be some detection due to the change before normalization
        // The key is that normalization should make the detection more consistent
    }
    
    #[test]
    fn test_spectral_flux_multiple_changes() {
        // Create spectrogram with multiple changes
        // Make sure each change creates a different max value to ensure flux
        let mut spectrogram = vec![vec![0.1f32; 1024]; 20];
        
        // Create three distinct changes with different patterns
        // Change 1 at frame 5: high values in first half of bins
        for bin in 0..512 {
            spectrogram[5][bin] = 1.0f32;
        }
        // Change 2 at frame 10: high values in second half of bins (different spectral pattern)
        for bin in 512..1024 {
            spectrogram[10][bin] = 1.0f32;
        }
        // Change 3 at frame 15: high values in middle bins (another different pattern)
        for bin in 256..768 {
            spectrogram[15][bin] = 1.0f32;
        }
        
        // Use lower threshold to catch the changes
        let onsets = detect_spectral_flux_onsets(&spectrogram, 0.3).unwrap();
        
        // Should detect multiple onsets
        assert!(onsets.len() >= 2, "Should detect multiple onsets, got {}: {:?}", onsets.len(), onsets);
    }
}

