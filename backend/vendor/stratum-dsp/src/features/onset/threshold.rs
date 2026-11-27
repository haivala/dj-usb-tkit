//! Adaptive thresholding utilities for onset detection
//!
//! Provides robust thresholding methods including median + MAD (Median Absolute Deviation)
//! as recommended by McFee & Ellis (2014) for improved onset detection robustness.
//!
//! NOTE: This module is currently **not wired into the main pipeline**; it is kept as a cited
//! reference implementation for future onset-threshold tuning work.

use crate::error::AnalysisError;

/// Compute adaptive threshold using median + MAD (Median Absolute Deviation)
///
/// This method is more robust to outliers than percentile-based thresholding.
/// It computes: `threshold = median(values) + k * MAD(values)`
/// where MAD = median(|values - median(values)|)
///
/// # Reference
///
/// McFee, B., & Ellis, D. P. W. (2014). Better Beat Tracking Through Robust Onset Aggregation.
/// *Proceedings of the International Society for Music Information Retrieval Conference*.
///
/// # Arguments
///
/// * `values` - Vector of flux/energy values to threshold
/// * `k` - Multiplier for MAD (typically 2.0-3.0, default 2.5)
///
/// # Returns
///
/// Adaptive threshold value
///
/// # Errors
///
/// Returns `AnalysisError` if values are empty
pub fn adaptive_threshold_median_mad(
    values: &[f32],
    k: f32,
) -> Result<f32, AnalysisError> {
    if values.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty values for threshold calculation".to_string()));
    }
    
    if k < 0.0 {
        return Err(AnalysisError::InvalidInput("MAD multiplier k must be non-negative".to_string()));
    }
    
    // Step 1: Compute median
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    
    let median = if sorted.len() % 2 == 0 {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) * 0.5
    } else {
        sorted[sorted.len() / 2]
    };
    
    // Step 2: Compute MAD (Median Absolute Deviation)
    // MAD = median(|values - median(values)|)
    let mut deviations: Vec<f32> = values.iter()
        .map(|&v| (v - median).abs())
        .collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    
    let mad = if deviations.len() % 2 == 0 {
        (deviations[deviations.len() / 2 - 1] + deviations[deviations.len() / 2]) * 0.5
    } else {
        deviations[deviations.len() / 2]
    };
    
    // Step 3: Compute threshold
    let threshold = median + k * mad;
    
    Ok(threshold)
}

/// Compute percentile-based threshold (existing method, kept for compatibility)
///
/// # Arguments
///
/// * `values` - Vector of flux/energy values to threshold
/// * `percentile` - Percentile (0.0-1.0), e.g., 0.8 for 80th percentile
///
/// # Returns
///
/// Threshold value at the specified percentile
///
/// # Errors
///
/// Returns `AnalysisError` if values are empty or percentile is out of range
pub fn percentile_threshold(
    values: &[f32],
    percentile: f32,
) -> Result<f32, AnalysisError> {
    if values.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty values for threshold calculation".to_string()));
    }
    
    if percentile < 0.0 || percentile > 1.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Percentile must be in [0.0, 1.0], got {}", percentile)
        ));
    }
    
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    
    let threshold_idx = ((sorted.len() as f32) * percentile) as usize;
    let threshold_idx = threshold_idx.min(sorted.len() - 1);
    
    Ok(sorted[threshold_idx])
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_adaptive_threshold_median_mad_basic() {
        // Test with simple values
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 100.0]; // Outlier at 100
        let threshold = adaptive_threshold_median_mad(&values, 2.5).unwrap();
        
        // Median is 3.5, MAD should be around 1.5
        // Threshold should be robust to the outlier
        assert!(threshold > 3.5);
        assert!(threshold < 50.0); // Should not be affected by outlier
    }
    
    #[test]
    fn test_adaptive_threshold_median_mad_empty() {
        let result = adaptive_threshold_median_mad(&[], 2.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_adaptive_threshold_median_mad_single_value() {
        let values = vec![5.0];
        let threshold = adaptive_threshold_median_mad(&values, 2.5).unwrap();
        assert_eq!(threshold, 5.0); // MAD of single value is 0
    }
    
    #[test]
    fn test_percentile_threshold_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let threshold = percentile_threshold(&values, 0.8).unwrap();
        
        // 80th percentile of [1,2,3,4,5] with 5 elements:
        // threshold_idx = (5 * 0.8) = 4, so sorted[4] = 5.0
        assert!((threshold - 5.0).abs() < 0.1);
        
        // Test 50th percentile (median)
        let threshold_50 = percentile_threshold(&values, 0.5).unwrap();
        assert!((threshold_50 - 3.0).abs() < 0.1);
    }
    
    #[test]
    fn test_percentile_threshold_empty() {
        let result = percentile_threshold(&[], 0.8);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_percentile_threshold_invalid_percentile() {
        let values = vec![1.0, 2.0, 3.0];
        assert!(percentile_threshold(&values, -0.1).is_err());
        assert!(percentile_threshold(&values, 1.1).is_err());
    }
}

