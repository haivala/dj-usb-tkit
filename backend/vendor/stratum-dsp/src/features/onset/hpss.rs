//! Harmonic-percussive source separation (HPSS) onset detection
//!
//! Separates harmonic and percussive components, detects onsets in percussive component.
//!
//! Algorithm:
//! 1. Apply horizontal median filter (across time) for harmonic component
//! 2. Apply vertical median filter (across frequency) for percussive component
//! 3. Iterate until convergence or max iterations
//! 4. Detect onsets in percussive component using energy flux
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::onset::hpss::{hpss_decompose, detect_hpss_onsets};
//!
//! let magnitude_spec = vec![vec![0.0f32; 1024]; 100];
//! let (harmonic, percussive) = hpss_decompose(&magnitude_spec, 10)?;
//! let onsets = detect_hpss_onsets(&percussive, 0.8)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Default number of iterations for HPSS decomposition
const DEFAULT_ITERATIONS: usize = 10;

/// Decompose spectrogram into harmonic and percussive components
///
/// Uses iterative median filtering to separate harmonic (sustained tones)
/// and percussive (transient sounds) components.
///
/// # Reference
///
/// Driedger, J., & Müller, M. (2014). Extending Harmonic-Percussive Separation of Audio Signals.
/// *Proceedings of the International Society for Music Information Retrieval Conference*.
///
/// # Arguments
///
/// * `magnitude_spec` - Magnitude spectrogram (n_frames × n_bins)
/// * `margin` - Median filter window margin (half-window size)
///   Typical values: 5-20 frames for horizontal, 5-20 bins for vertical
///
/// # Returns
///
/// Tuple of (harmonic, percussive) spectrograms with same dimensions as input
///
/// # Errors
///
/// Returns `AnalysisError` if spectrogram is empty or frames have inconsistent lengths
///
/// # Algorithm
///
/// Iterative algorithm:
/// 1. Initialize harmonic and percussive with input spectrogram
/// 2. For each iteration:
///    - Apply horizontal median filter (across time) to harmonic
///    - Apply vertical median filter (across frequency) to percussive
///    - Reconstruct: harmonic + percussive = original
/// 3. Return separated components
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::onset::hpss::hpss_decompose;
///
/// let magnitude_spec = vec![vec![0.5f32; 1024]; 100];
/// let (harmonic, percussive) = hpss_decompose(&magnitude_spec, 10)?;
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn hpss_decompose(
    magnitude_spec: &[Vec<f32>],
    margin: usize,
) -> Result<(Vec<Vec<f32>>, Vec<Vec<f32>>), AnalysisError> {
    // Validate inputs
    if magnitude_spec.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty spectrogram".to_string()));
    }
    
    let n_frames = magnitude_spec.len();
    let n_bins = magnitude_spec[0].len();
    
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty frames".to_string()));
    }
    
    // Check that all frames have the same length
    for (i, frame) in magnitude_spec.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!("Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                        n_bins, i, frame.len())
            ));
        }
    }
    
    log::debug!("HPSS decomposition: {} frames, {} bins, margin={}, iterations={}",
                n_frames, n_bins, margin, DEFAULT_ITERATIONS);
    
    // Initialize harmonic and percussive with input spectrogram
    let mut harmonic: Vec<Vec<f32>> = magnitude_spec.iter()
        .map(|frame| frame.clone())
        .collect();
    let mut percussive: Vec<Vec<f32>> = magnitude_spec.iter()
        .map(|frame| frame.clone())
        .collect();
    
    // Iterative refinement
    for iteration in 0..DEFAULT_ITERATIONS {
        // Store previous values for convergence check
        let harmonic_prev = harmonic.clone();
        let percussive_prev = percussive.clone();
        
        // Apply horizontal median filter (across time) for harmonic
        // This smooths across time, preserving frequency structure
        let harmonic_filtered = apply_horizontal_median_filter(&harmonic, margin);
        
        // Apply vertical median filter (across frequency) for percussive
        // This smooths across frequency, preserving temporal structure
        let percussive_filtered = apply_vertical_median_filter(&percussive, margin);
        
        // Update components
        harmonic = harmonic_filtered;
        percussive = percussive_filtered;
        
        // Reconstruct: ensure harmonic + percussive = original
        // This is done by soft masking: each component gets a portion based on its strength
        for frame_idx in 0..n_frames {
            for bin_idx in 0..n_bins {
                let original = magnitude_spec[frame_idx][bin_idx];
                let h = harmonic[frame_idx][bin_idx];
                let p = percussive[frame_idx][bin_idx];
                
                // Soft mask: distribute energy based on relative strength
                let total = h + p;
                if total > 1e-10 {
                    let h_ratio = h / total;
                    let p_ratio = p / total;
                    harmonic[frame_idx][bin_idx] = original * h_ratio;
                    percussive[frame_idx][bin_idx] = original * p_ratio;
                } else {
                    harmonic[frame_idx][bin_idx] = original * 0.5;
                    percussive[frame_idx][bin_idx] = original * 0.5;
                }
            }
        }
        
        // Check for convergence (simple check: if change is small, we can stop early)
        if iteration > 0 {
            let mut max_change = 0.0f32;
            for frame_idx in 0..n_frames {
                for bin_idx in 0..n_bins {
                    let h_change = (harmonic[frame_idx][bin_idx] - harmonic_prev[frame_idx][bin_idx]).abs();
                    let p_change = (percussive[frame_idx][bin_idx] - percussive_prev[frame_idx][bin_idx]).abs();
                    max_change = max_change.max(h_change).max(p_change);
                }
            }
            
            if max_change < 1e-6 {
                log::debug!("HPSS converged after {} iterations", iteration + 1);
                break;
            }
        }
    }
    
    log::debug!("HPSS decomposition complete");
    
    Ok((harmonic, percussive))
}

/// Apply horizontal median filter (across time)
/// For each frequency bin, compute median across time frames
fn apply_horizontal_median_filter(
    spectrogram: &[Vec<f32>],
    margin: usize,
) -> Vec<Vec<f32>> {
    let n_frames = spectrogram.len();
    let n_bins = spectrogram[0].len();
    let mut filtered = vec![vec![0.0f32; n_bins]; n_frames];
    
    for bin_idx in 0..n_bins {
        for frame_idx in 0..n_frames {
            // Collect values in window around current frame
            let start = frame_idx.saturating_sub(margin);
            let end = (frame_idx + margin + 1).min(n_frames);
            
            let mut window: Vec<f32> = (start..end)
                .map(|f| spectrogram[f][bin_idx])
                .collect();
            
            // Compute median
            window.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = if window.is_empty() {
                0.0
            } else if window.len() % 2 == 0 {
                (window[window.len() / 2 - 1] + window[window.len() / 2]) * 0.5
            } else {
                window[window.len() / 2]
            };
            
            filtered[frame_idx][bin_idx] = median;
        }
    }
    
    filtered
}

/// Apply vertical median filter (across frequency)
/// For each time frame, compute median across frequency bins
fn apply_vertical_median_filter(
    spectrogram: &[Vec<f32>],
    margin: usize,
) -> Vec<Vec<f32>> {
    let n_frames = spectrogram.len();
    let n_bins = spectrogram[0].len();
    let mut filtered = vec![vec![0.0f32; n_bins]; n_frames];
    
    for frame_idx in 0..n_frames {
        for bin_idx in 0..n_bins {
            // Collect values in window around current bin
            let start = bin_idx.saturating_sub(margin);
            let end = (bin_idx + margin + 1).min(n_bins);
            
            let mut window: Vec<f32> = (start..end)
                .map(|b| spectrogram[frame_idx][b])
                .collect();
            
            // Compute median
            window.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = if window.is_empty() {
                0.0
            } else if window.len() % 2 == 0 {
                (window[window.len() / 2 - 1] + window[window.len() / 2]) * 0.5
            } else {
                window[window.len() / 2]
            };
            
            filtered[frame_idx][bin_idx] = median;
        }
    }
    
    filtered
}

/// Detect onsets in percussive component
///
/// Uses energy flux method on the percussive spectrogram to detect onsets.
/// The percussive component contains transient sounds (drums, clicks) which
/// are ideal for onset detection.
///
/// # Arguments
///
/// * `percussive_component` - Percussive spectrogram from HPSS decomposition
/// * `threshold_percentile` - Threshold percentile (0.0-1.0)
///   e.g., 0.8 means threshold is 80th percentile of flux values
///
/// # Returns
///
/// Vector of onset frame indices (0-indexed), sorted by frame number
///
/// # Errors
///
/// Returns `AnalysisError` if percussive component is invalid
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::onset::hpss::{hpss_decompose, detect_hpss_onsets};
///
/// let magnitude_spec = vec![vec![0.5f32; 1024]; 100];
/// let (_, percussive) = hpss_decompose(&magnitude_spec, 10)?;
/// let onsets = detect_hpss_onsets(&percussive, 0.8)?;
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn detect_hpss_onsets(
    percussive_component: &[Vec<f32>],
    threshold_percentile: f32,
) -> Result<Vec<usize>, AnalysisError> {
    // Validate inputs
    if percussive_component.is_empty() {
        return Ok(Vec::new());
    }
    
    if threshold_percentile < 0.0 || threshold_percentile > 1.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Threshold percentile must be in [0, 1], got {}", threshold_percentile)
        ));
    }
    
    if percussive_component.len() < 2 {
        // Need at least 2 frames to compute flux
        return Ok(Vec::new());
    }
    
    log::debug!("Detecting HPSS onsets: {} frames, threshold_percentile={:.2}",
                percussive_component.len(), threshold_percentile);
    
    // Compute energy per frame (sum of squared magnitudes)
    let mut frame_energies = Vec::with_capacity(percussive_component.len());
    
    for frame in percussive_component {
        let energy: f32 = frame.iter().map(|&x| x * x).sum();
        frame_energies.push(energy);
    }
    
    // Compute energy flux (derivative)
    // E_flux[n] = max(0, E[n] - E[n-1])
    let mut energy_flux = Vec::with_capacity(frame_energies.len() - 1);
    
    for i in 1..frame_energies.len() {
        let flux = (frame_energies[i] - frame_energies[i - 1]).max(0.0);
        energy_flux.push(flux);
    }
    
    if energy_flux.is_empty() {
        return Ok(Vec::new());
    }
    
    // Apply percentile-based threshold
    let mut sorted_flux = energy_flux.clone();
    sorted_flux.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    
    let threshold_idx = ((sorted_flux.len() as f32) * threshold_percentile) as usize;
    let threshold_idx = threshold_idx.min(sorted_flux.len() - 1);
    let threshold = sorted_flux[threshold_idx];
    
    log::debug!("HPSS energy flux: max={:.6}, threshold={:.6} ({}th percentile)",
                sorted_flux.last().unwrap_or(&0.0), threshold, threshold_percentile);
    
    // Peak-pick to find local maxima above threshold
    let mut onsets = Vec::new();
    
    for i in 1..(energy_flux.len() - 1) {
        let flux = energy_flux[i];
        let prev_flux = energy_flux[i - 1];
        let next_flux = energy_flux[i + 1];
        
        // Check if this is a local maximum above threshold
        if flux > threshold && flux > prev_flux && flux >= next_flux {
            // Frame index is i+1 (since flux[i] is between frame i and i+1)
            onsets.push(i + 1);
        }
    }
    
    // Handle edge cases
    if energy_flux.len() > 1 && 
       energy_flux[0] > threshold && 
       energy_flux[0] >= energy_flux[1] {
        onsets.push(1);
    }
    
    let last_idx = energy_flux.len() - 1;
    if energy_flux.len() > 1 && 
       energy_flux[last_idx] > threshold && 
       energy_flux[last_idx] > energy_flux[last_idx - 1] {
        onsets.push(energy_flux.len());
    }
    
    // Sort and deduplicate
    onsets.sort_unstable();
    onsets.dedup();
    
    log::debug!("HPSS detected {} onsets", onsets.len());
    
    Ok(onsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_hpss_decompose_basic() {
        // Create a simple spectrogram
        let magnitude_spec = vec![vec![0.5f32; 1024]; 10];
        
        let (harmonic, percussive) = hpss_decompose(&magnitude_spec, 5).unwrap();
        
        // Should have same dimensions
        assert_eq!(harmonic.len(), magnitude_spec.len());
        assert_eq!(percussive.len(), magnitude_spec.len());
        assert_eq!(harmonic[0].len(), magnitude_spec[0].len());
        assert_eq!(percussive[0].len(), magnitude_spec[0].len());
        
        // Harmonic + percussive should approximately equal original
        for frame_idx in 0..magnitude_spec.len() {
            for bin_idx in 0..magnitude_spec[0].len() {
                let original = magnitude_spec[frame_idx][bin_idx];
                let h = harmonic[frame_idx][bin_idx];
                let p = percussive[frame_idx][bin_idx];
                let reconstructed = h + p;
                
                // Allow some tolerance due to iterative approximation
                assert!((reconstructed - original).abs() < 0.1,
                        "Reconstruction error at frame {}, bin {}: expected {}, got {}",
                        frame_idx, bin_idx, original, reconstructed);
            }
        }
    }
    
    #[test]
    fn test_hpss_decompose_empty() {
        let magnitude_spec = vec![];
        let result = hpss_decompose(&magnitude_spec, 5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_hpss_decompose_inconsistent_lengths() {
        let mut magnitude_spec = vec![vec![0.5f32; 1024]; 10];
        magnitude_spec[5] = vec![0.5f32; 512];
        
        let result = hpss_decompose(&magnitude_spec, 5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_hpss_decompose_harmonic_vs_percussive() {
        // Create spectrogram with clear harmonic (sustained) and percussive (transient) components
        let mut magnitude_spec = vec![vec![0.0f32; 1024]; 20];
        
        // Harmonic: sustained energy across time in specific frequency bins
        for frame_idx in 0..20 {
            for bin_idx in 100..200 {
                magnitude_spec[frame_idx][bin_idx] = 0.8f32;
            }
        }
        
        // Percussive: transient bursts at specific frames
        for frame_idx in [5, 10, 15] {
            for bin_idx in 0..1024 {
                magnitude_spec[frame_idx][bin_idx] = 1.0f32;
            }
        }
        
        let (_harmonic, percussive) = hpss_decompose(&magnitude_spec, 3).unwrap();
        
        // Harmonic should have sustained energy
        // Percussive should have transient bursts
        // Check that percussive has higher energy at transient frames
        let percussive_energy_frame_5: f32 = percussive[5].iter().map(|&x| x * x).sum();
        let percussive_energy_frame_3: f32 = percussive[3].iter().map(|&x| x * x).sum();
        
        assert!(percussive_energy_frame_5 > percussive_energy_frame_3,
                "Percussive should have higher energy at transient frames");
    }
    
    #[test]
    fn test_detect_hpss_onsets_basic() {
        // Create percussive component with clear onsets
        let mut percussive = vec![vec![0.01f32; 1024]; 20];
        
        // Add transient bursts at frames 5, 10, 15
        for frame_idx in [5, 10, 15] {
            for bin_idx in 0..1024 {
                percussive[frame_idx][bin_idx] = 1.0f32;
            }
        }
        
        let onsets = detect_hpss_onsets(&percussive, 0.5).unwrap();
        
        // Should detect onsets near the transient frames
        assert!(!onsets.is_empty(), "Should detect at least one onset");
        assert!(onsets.len() >= 2, "Should detect multiple onsets, got {}", onsets.len());
    }
    
    #[test]
    fn test_detect_hpss_onsets_empty() {
        let percussive = vec![];
        let onsets = detect_hpss_onsets(&percussive, 0.8).unwrap();
        assert!(onsets.is_empty());
    }
    
    #[test]
    fn test_detect_hpss_onsets_single_frame() {
        let percussive = vec![vec![0.5f32; 1024]];
        let onsets = detect_hpss_onsets(&percussive, 0.8).unwrap();
        // Need at least 2 frames
        assert!(onsets.is_empty());
    }
    
    #[test]
    fn test_detect_hpss_onsets_invalid_percentile() {
        let percussive = vec![vec![0.5f32; 1024]; 10];
        
        let result = detect_hpss_onsets(&percussive, -0.1);
        assert!(result.is_err());
        
        let result = detect_hpss_onsets(&percussive, 1.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_detect_hpss_onsets_threshold_sensitivity() {
        // Create percussive with varying energy
        let mut percussive = vec![vec![0.01f32; 1024]; 20];
        for i in 0..20 {
            let amplitude = 0.1 + (i as f32 / 20.0) * 0.9;
            for bin_idx in 0..1024 {
                percussive[i][bin_idx] = amplitude;
            }
        }
        
        let onsets_low = detect_hpss_onsets(&percussive, 0.5).unwrap();
        let onsets_high = detect_hpss_onsets(&percussive, 0.9).unwrap();
        
        assert!(onsets_low.len() >= onsets_high.len(),
                "Lower threshold should detect more onsets: {} vs {}",
                onsets_low.len(), onsets_high.len());
    }
}

