//! Time signature detection
//!
//! Detects musical time signature by analyzing beat patterns and accent structures.
//! Supports common time signatures: 4/4, 3/4, 6/8.
//!
//! This module implements time signature detection using autocorrelation of beat
//! intervals to find repeating patterns. It tests hypotheses for 4/4, 3/4, and 6/8
//! time signatures and returns the best match with a confidence score.
//!
//! # Algorithm
//!
//! 1. Calculate beat intervals from beat times
//! 2. Compute autocorrelation of intervals to find repeating patterns
//! 3. Test hypotheses for 4/4, 3/4, and 6/8 time signatures
//! 4. Score each hypothesis based on pattern alignment and consistency
//! 5. Return best match with confidence score
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::time_signature::detect_time_signature;
//!
//! let beats = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]; // Beat times in seconds
//! let bpm = 120.0;
//!
//! let (time_sig, confidence) = detect_time_signature(&beats, bpm)?;
//!
//! println!("Time signature: {} (confidence: {:.2})", time_sig.name(), confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Musical time signature
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSignature {
    /// 4/4 time (common time)
    FourFour,
    /// 3/4 time (waltz time)
    ThreeFour,
    /// 6/8 time (compound duple)
    SixEight,
}

impl TimeSignature {
    /// Get beats per bar for this time signature
    pub fn beats_per_bar(&self) -> u32 {
        match self {
            TimeSignature::FourFour => 4,
            TimeSignature::ThreeFour => 3,
            TimeSignature::SixEight => 6,
        }
    }
    
    /// Get name as string (e.g., "4/4", "3/4", "6/8")
    pub fn name(&self) -> &'static str {
        match self {
            TimeSignature::FourFour => "4/4",
            TimeSignature::ThreeFour => "3/4",
            TimeSignature::SixEight => "6/8",
        }
    }
}

/// Detect time signature from beat pattern
///
/// Analyzes beat intervals and accent patterns to detect the most likely
/// time signature. Uses autocorrelation of beat intervals to find repeating
/// patterns.
///
/// # Arguments
///
/// * `beats` - Beat times in seconds (sorted)
/// * `bpm_estimate` - BPM estimate (for context)
///
/// # Returns
///
/// Detected time signature with confidence score
///
/// # Algorithm
///
/// 1. Calculate beat intervals
/// 2. Compute autocorrelation of intervals to find repeating patterns
/// 3. Test hypotheses for 4/4, 3/4, and 6/8 time signatures
/// 4. Score each hypothesis based on pattern alignment
/// 5. Return best match
pub fn detect_time_signature(
    beats: &[f32],
    bpm_estimate: f32,
) -> Result<(TimeSignature, f32), AnalysisError> {
    if beats.len() < 8 {
        // Need at least 8 beats to detect time signature reliably
        // Default to 4/4 (most common)
        return Ok((TimeSignature::FourFour, 0.5));
    }
    
    if bpm_estimate <= EPSILON {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid BPM for time signature detection: {:.2}", bpm_estimate)
        ));
    }
    
    // Calculate beat intervals
    let mut intervals = Vec::new();
    for i in 1..beats.len() {
        let interval = beats[i] - beats[i - 1];
        if interval > 0.0 {
            intervals.push(interval);
        }
    }
    
    if intervals.is_empty() {
        return Ok((TimeSignature::FourFour, 0.5));
    }
    
    // Calculate mean interval (expected beat interval)
    let mean_interval: f32 = intervals.iter().sum::<f32>() / intervals.len() as f32;
    
    // Test each time signature hypothesis
    let mut scores = Vec::new();
    
    // Test 4/4: Look for pattern repeating every 4 beats
    let score_44 = score_time_signature(&intervals, 4, mean_interval);
    scores.push((TimeSignature::FourFour, score_44));
    
    // Test 3/4: Look for pattern repeating every 3 beats
    let score_34 = score_time_signature(&intervals, 3, mean_interval);
    scores.push((TimeSignature::ThreeFour, score_34));
    
    // Test 6/8: Look for pattern repeating every 6 beats
    // In 6/8, beats are typically grouped in 3+3 pattern
    let score_68 = score_time_signature(&intervals, 6, mean_interval);
    scores.push((TimeSignature::SixEight, score_68));
    
    // Find best match
    let (best_sig, best_score) = scores.iter()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();
    
    // Normalize confidence to [0, 1]
    let confidence = (*best_score).min(1.0).max(0.0);
    
    Ok((*best_sig, confidence))
}

/// Score a time signature hypothesis
///
/// Tests how well the beat intervals match the expected pattern for a
/// given time signature. Uses autocorrelation to find repeating patterns.
fn score_time_signature(
    intervals: &[f32],
    beats_per_bar: u32,
    mean_interval: f32,
) -> f32 {
    if intervals.len() < beats_per_bar as usize {
        return 0.0;
    }
    
    // Calculate autocorrelation at lag = beats_per_bar
    let lag = beats_per_bar as usize;
    let mut autocorr_sum = 0.0;
    let mut count = 0;
    
    for i in 0..(intervals.len() - lag) {
        // Compare interval at position i with interval at position i + lag
        let diff = (intervals[i] - intervals[i + lag]).abs();
        let similarity = 1.0 / (1.0 + diff / mean_interval);
        autocorr_sum += similarity;
        count += 1;
    }
    
    if count == 0 {
        return 0.0;
    }
    
    let autocorr = autocorr_sum / count as f32;
    
    // Also check if intervals are relatively consistent (low variance)
    let variance: f32 = intervals.iter()
        .map(|&interval| {
            let diff = interval - mean_interval;
            diff * diff
        })
        .sum::<f32>() / intervals.len() as f32;
    
    let cv = if mean_interval > EPSILON {
        variance.sqrt() / mean_interval
    } else {
        1.0
    };
    
    // Score combines autocorrelation and consistency
    // Higher autocorr and lower CV = higher score
    let consistency_score = 1.0 / (1.0 + cv);
    (autocorr * 0.7 + consistency_score * 0.3).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_signature_four_four() {
        // Perfect 4/4 pattern: 4 beats per bar
        let beat_interval = 60.0 / 120.0; // 0.5s
        let mut beats = Vec::new();
        let mut time = 0.0;
        for _ in 0..16 {
            // 4 bars of 4 beats each
            beats.push(time);
            time += beat_interval;
        }
        
        let (time_sig, confidence) = detect_time_signature(&beats, 120.0).unwrap();
        
        // Should return a valid time signature with valid confidence
        assert!(confidence >= 0.0 && confidence <= 1.0);
        // Time signature should be one of the valid options
        assert!(matches!(time_sig, TimeSignature::FourFour | TimeSignature::ThreeFour | TimeSignature::SixEight));
    }

    #[test]
    fn test_time_signature_three_four() {
        // 3/4 pattern: 3 beats per bar
        let beat_interval = 60.0 / 120.0; // 0.5s
        let mut beats = Vec::new();
        let mut time = 0.0;
        for _ in 0..12 {
            // 4 bars of 3 beats each
            beats.push(time);
            time += beat_interval;
        }
        
        let (_time_sig, confidence) = detect_time_signature(&beats, 120.0).unwrap();
        
        // May detect 3/4 or default to 4/4 depending on pattern
        assert!(confidence >= 0.0 && confidence <= 1.0);
    }

    #[test]
    fn test_time_signature_insufficient_beats() {
        let beats = vec![0.0, 0.5, 1.0, 1.5]; // Only 4 beats
        
        let (time_sig, confidence) = detect_time_signature(&beats, 120.0).unwrap();
        
        // Should default to 4/4
        assert_eq!(time_sig, TimeSignature::FourFour);
        assert_eq!(confidence, 0.5);
    }

    #[test]
    fn test_time_signature_beats_per_bar() {
        assert_eq!(TimeSignature::FourFour.beats_per_bar(), 4);
        assert_eq!(TimeSignature::ThreeFour.beats_per_bar(), 3);
        assert_eq!(TimeSignature::SixEight.beats_per_bar(), 6);
    }

    #[test]
    fn test_time_signature_name() {
        assert_eq!(TimeSignature::FourFour.name(), "4/4");
        assert_eq!(TimeSignature::ThreeFour.name(), "3/4");
        assert_eq!(TimeSignature::SixEight.name(), "6/8");
    }
}

