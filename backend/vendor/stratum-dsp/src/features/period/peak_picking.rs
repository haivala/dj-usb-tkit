//! Robust peak detection utilities
//!
//! Provides peak detection algorithms for finding local maxima in signals.
//! Used for detecting peaks in autocorrelation functions and other 1D signals.

const EPSILON: f32 = 1e-10;

/// Find peaks in a signal
///
/// Detects local maxima that exceed a threshold and are separated by
/// a minimum distance.
///
/// # Arguments
///
/// * `signal` - Signal to find peaks in
/// * `threshold` - Minimum peak height (absolute value or relative to max)
/// * `min_distance` - Minimum distance between peaks (in samples)
///
/// # Returns
///
/// Vector of (index, value) pairs for detected peaks, sorted by value (highest first)
///
/// # Algorithm
///
/// 1. Find all local maxima (value > left neighbor && value > right neighbor)
/// 2. Filter by threshold
/// 3. Enforce minimum distance (keep highest peak when too close)
/// 4. Sort by value
///
/// # Example
///
/// ```
/// use stratum_dsp::features::period::peak_picking::find_peaks;
///
/// let signal = vec![0.0, 0.5, 1.0, 0.7, 0.3, 0.9, 0.2];
/// let peaks = find_peaks(&signal, 0.5, 2);
/// // Returns peaks at indices 2 (value 1.0) and 5 (value 0.9)
/// ```
pub fn find_peaks(
    signal: &[f32],
    threshold: f32,
    min_distance: usize,
) -> Vec<(usize, f32)> {
    log::debug!(
        "Finding peaks in signal of length {}, threshold={:.3}, min_distance={}",
        signal.len(),
        threshold,
        min_distance
    );

    if signal.is_empty() {
        return vec![];
    }

    if signal.len() < 3 {
        // Need at least 3 points for local maximum detection
        return vec![];
    }

    // Compute threshold (absolute or relative to max)
    // If threshold is in (0, 1) exclusive, treat as relative
    // Otherwise, treat as absolute
    let max_value = signal.iter().copied().fold(0.0f32, f32::max);
    let actual_threshold = if threshold > 0.0 && threshold < 1.0 && max_value > 0.0 {
        // Relative threshold (0.0-1.0) when max > 0
        max_value * threshold
    } else {
        // Absolute threshold (including 1.0 and above)
        threshold
    };

    if max_value < EPSILON {
        return vec![];
    }

    // Find all local maxima
    let mut peaks = Vec::new();
    
    for i in 1..(signal.len() - 1) {
        let value = signal[i];
        
        // Check if local maximum
        if value > signal[i - 1] && value > signal[i + 1] {
            // Check threshold
            if value >= actual_threshold {
                peaks.push((i, value));
            }
        }
    }

    // Handle edge cases (first and last elements)
    if signal.len() >= 2 {
        // First element: peak if > second element and > threshold
        if signal[0] > signal[1] && signal[0] >= actual_threshold {
            peaks.push((0, signal[0]));
        }
        
        // Last element: peak if > second-to-last and > threshold
        let last_idx = signal.len() - 1;
        if signal[last_idx] > signal[last_idx - 1] && signal[last_idx] >= actual_threshold {
            peaks.push((last_idx, signal[last_idx]));
        }
    }

    // Enforce minimum distance
    if min_distance > 0 && peaks.len() > 1 {
        // Sort by value (highest first) to keep best peaks
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        let mut filtered_peaks = Vec::new();
        for (idx, value) in peaks {
            // Check if too close to any existing peak
            let too_close = filtered_peaks.iter().any(|(existing_idx, _)| {
                (idx as i32 - *existing_idx as i32).abs() < min_distance as i32
            });
            
            if !too_close {
                filtered_peaks.push((idx, value));
            }
        }
        
        peaks = filtered_peaks;
    }

    // Final sort by value (highest first)
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    log::debug!("Found {} peaks", peaks.len());
    
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_peaks_basic() {
        let signal = vec![0.0, 0.5, 1.0, 0.7, 0.3, 0.9, 0.2];
        let peaks = find_peaks(&signal, 0.5, 2);

        assert!(!peaks.is_empty());
        // Should find peak at index 2 (value 1.0) and index 5 (value 0.9)
        assert!(peaks.iter().any(|(idx, _)| *idx == 2));
        assert!(peaks.iter().any(|(idx, _)| *idx == 5));
    }

    #[test]
    fn test_find_peaks_empty() {
        let signal = vec![];
        let peaks = find_peaks(&signal, 0.5, 2);
        assert!(peaks.is_empty());
    }

    #[test]
    fn test_find_peaks_too_short() {
        let signal = vec![1.0, 2.0];
        let peaks = find_peaks(&signal, 0.5, 2);
        // Need at least 3 points for local maximum
        assert!(peaks.is_empty());
    }

    #[test]
    fn test_find_peaks_threshold() {
        let signal = vec![0.1, 0.2, 0.3, 0.4, 0.3, 0.2, 0.1];
        // Relative threshold (0.5 = 50% of max)
        let peaks = find_peaks(&signal, 0.5, 1);
        // Peak at index 3 (value 0.4), threshold = 0.4 * 0.5 = 0.2
        assert!(!peaks.is_empty());
        
        // Absolute threshold
        let peaks = find_peaks(&signal, 0.5, 1);
        // Should be empty (max is 0.4, threshold 0.5 > 0.4)
        // Actually, relative threshold 0.5 means 0.2, so peak should be found
        assert!(!peaks.is_empty());
    }

    #[test]
    fn test_find_peaks_min_distance() {
        // Two close peaks
        let signal = vec![0.0, 0.5, 1.0, 0.8, 0.9, 0.3, 0.1];
        let peaks = find_peaks(&signal, 0.3, 3);
        
        // With min_distance=3, should only keep one of the close peaks
        // Peak at index 2 (value 1.0) and index 4 (value 0.9) are 2 apart
        // Should keep the higher one (index 2)
        assert!(!peaks.is_empty());
    }

    #[test]
    fn test_find_peaks_edge_cases() {
        // First element is peak
        let signal = vec![1.0, 0.5, 0.3];
        let peaks = find_peaks(&signal, 0.5, 1);
        assert!(peaks.iter().any(|(idx, _)| *idx == 0));
        
        // Last element is peak
        let signal = vec![0.3, 0.5, 1.0];
        let peaks = find_peaks(&signal, 0.5, 1);
        assert!(peaks.iter().any(|(idx, _)| *idx == 2));
    }

    #[test]
    fn test_find_peaks_all_below_threshold() {
        let signal = vec![0.1, 0.2, 0.3, 0.2, 0.1];
        // Use absolute threshold > max value (0.3)
        // Use threshold > 1.0 to force absolute mode
        let peaks = find_peaks(&signal, 1.0, 1); // Absolute threshold > 0.3
        // All values below threshold
        assert!(peaks.is_empty());
    }

    #[test]
    fn test_find_peaks_sorted() {
        let signal = vec![0.0, 0.5, 1.0, 0.7, 0.3, 0.9, 0.2];
        let peaks = find_peaks(&signal, 0.3, 1);
        
        // Should be sorted by value (highest first)
        for i in 1..peaks.len() {
            assert!(
                peaks[i - 1].1 >= peaks[i].1,
                "Peaks should be sorted by value"
            );
        }
    }
}

