//! Key clarity scoring
//!
//! Estimates how "tonal" vs "atonal" a track is by analyzing the distribution
//! of key detection scores.
//!
//! # Reference
//!
//! Krumhansl, C. L., & Kessler, E. J. (1982). Tracing the Dynamic Changes in Perceived
//! Tonal Organization in a Spatial Representation of Musical Keys. *Psychological Review*,
//! 89(4), 334-368.

/// Compute key clarity from key scores
///
/// Key clarity measures how "tonal" vs "atonal" the track is. High clarity
/// indicates a clear tonal center (reliable key detection), while low clarity
/// indicates ambiguous or atonal music (unreliable key detection).
///
/// Formula: `clarity = (best_score - average_score) / range`
///
/// Where:
/// - `best_score`: Highest key score
/// - `average_score`: Mean of all 24 key scores
/// - `range`: Difference between max and min scores
///
/// # Arguments
///
/// * `scores` - All 24 key scores (ranked, highest first)
///   Should be a slice of `(Key, f32)` tuples
///
/// # Returns
///
/// Clarity score (0.0-1.0), where:
/// - **High (>0.5)**: Strong tonality, reliable key detection
/// - **Medium (0.2-0.5)**: Moderate tonality
/// - **Low (<0.2)**: Weak tonality, key detection may be unreliable
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::key::key_clarity::compute_key_clarity;
/// use stratum_dsp::analysis::result::Key;
///
/// let scores = vec![
///     (Key::Major(0), 0.8),
///     (Key::Major(7), 0.6),
///     // ... other keys
/// ];
/// let clarity = compute_key_clarity(&scores);
/// println!("Key clarity: {:.2}", clarity);
/// ```
pub fn compute_key_clarity(scores: &[(crate::analysis::result::Key, f32)]) -> f32 {
    log::debug!("Computing key clarity from {} scores", scores.len());
    
    if scores.is_empty() {
        return 0.0;
    }
    
    if scores.len() < 2 {
        // Only one score - clarity is undefined, return 0
        return 0.0;
    }
    
    // Extract just the scores (ignore keys)
    let score_values: Vec<f32> = scores.iter().map(|(_, score)| *score).collect();
    
    // Find best (highest) score
    let best_score = score_values[0];
    
    // Compute average
    let sum: f32 = score_values.iter().sum();
    let average_score = sum / score_values.len() as f32;
    
    // Find range (max - min)
    let min_score = *score_values.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let max_score = *score_values.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let range = max_score - min_score;
    
    // Compute clarity: (best - average) / range
    if range > 1e-10 {
        let clarity = (best_score - average_score) / range;
        // Clamp to [0, 1]
        clarity.max(0.0).min(1.0)
    } else {
        // All scores are the same - no clarity
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::result::Key;
    
    #[test]
    fn test_compute_key_clarity_empty() {
        let clarity = compute_key_clarity(&[]);
        assert_eq!(clarity, 0.0);
    }
    
    #[test]
    fn test_compute_key_clarity_single() {
        let scores = vec![(Key::Major(0), 0.8)];
        let clarity = compute_key_clarity(&scores);
        assert_eq!(clarity, 0.0);
    }
    
    #[test]
    fn test_compute_key_clarity_high() {
        // High clarity: one score is much higher than others
        let scores = vec![
            (Key::Major(0), 0.9),
            (Key::Major(1), 0.3),
            (Key::Major(2), 0.3),
            (Key::Major(3), 0.3),
        ];
        let clarity = compute_key_clarity(&scores);
        assert!(clarity > 0.5, "High clarity should be > 0.5");
    }
    
    #[test]
    fn test_compute_key_clarity_low() {
        // Low clarity: scores are similar
        let scores = vec![
            (Key::Major(0), 0.5),
            (Key::Major(1), 0.48),
            (Key::Major(2), 0.49),
            (Key::Major(3), 0.47),
        ];
        let clarity = compute_key_clarity(&scores);
        assert!(clarity < 0.5, "Low clarity should be < 0.5");
    }
    
    #[test]
    fn test_compute_key_clarity_all_same() {
        // All scores the same - no clarity
        let scores = vec![
            (Key::Major(0), 0.5),
            (Key::Major(1), 0.5),
            (Key::Major(2), 0.5),
        ];
        let clarity = compute_key_clarity(&scores);
        assert_eq!(clarity, 0.0);
    }
    
    #[test]
    fn test_compute_key_clarity_clamped() {
        // Test that clarity is clamped to [0, 1]
        let scores = vec![
            (Key::Major(0), 1.0),
            (Key::Major(1), 0.0),
        ];
        let clarity = compute_key_clarity(&scores);
        assert!(clarity >= 0.0 && clarity <= 1.0);
    }
}
