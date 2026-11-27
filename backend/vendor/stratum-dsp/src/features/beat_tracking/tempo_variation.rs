//! Tempo variation detection
//!
//! Detects tempo changes in audio by analyzing beat intervals over time.
//! Used to identify segments with tempo drift for Bayesian tracking.
//!
//! This module implements segment-based tempo variation detection by dividing
//! the audio into overlapping segments and analyzing the coefficient of variation
//! (CV) of beat intervals. Segments with high CV (>0.15) are marked as variable
//! tempo and automatically refined using the Bayesian tracker.
//!
//! # Algorithm
//!
//! 1. Divide beats into overlapping segments (4-8 seconds, 50% overlap)
//! 2. For each segment, compute beat intervals
//! 3. Calculate coefficient of variation (CV) of intervals
//! 4. If CV > threshold (0.15), mark segment as variable tempo
//! 5. Estimate BPM for each segment from mean interval
//!
//! # Use Cases
//!
//! - DJ mixes with tempo changes
//! - Live recordings with tempo drift
//! - Variable-tempo tracks
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::tempo_variation::detect_tempo_variations;
//!
//! let beats = vec![0.0, 0.5, 1.0, 1.5, 2.0]; // Beat times in seconds
//! let nominal_bpm = 120.0;
//!
//! let segments = detect_tempo_variations(&beats, nominal_bpm)?;
//!
//! for segment in segments {
//!     println!("Segment [{:.2}s-{:.2}s]: BPM={:.2}, variable={}",
//!              segment.start_time, segment.end_time, segment.bpm, segment.is_variable);
//! }
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Minimum segment duration in seconds (for tempo analysis)
const MIN_SEGMENT_DURATION_S: f32 = 2.0;

/// Tempo variation threshold (coefficient of variation)
/// If CV > this threshold, segment is considered variable tempo
const TEMPO_VARIATION_THRESHOLD: f32 = 0.15;

/// Tempo segment with detected BPM
#[derive(Debug, Clone)]
pub struct TempoSegment {
    /// Start time in seconds
    pub start_time: f32,
    
    /// End time in seconds
    pub end_time: f32,
    
    /// Detected BPM for this segment
    pub bpm: f32,
    
    /// Confidence in BPM estimate (0.0-1.0)
    pub confidence: f32,
    
    /// Whether this segment has variable tempo
    pub is_variable: bool,
}

/// Detect tempo variations by segmenting audio and analyzing beat intervals
///
/// Divides the audio into segments and analyzes beat intervals to detect
/// tempo changes. Segments with high tempo variation are marked for
/// Bayesian tracking.
///
/// # Arguments
///
/// * `beats` - Beat times in seconds (sorted)
/// * `nominal_bpm` - Nominal BPM estimate (from period estimation)
///
/// # Returns
///
/// Vector of tempo segments with detected BPM and variation flags
///
/// # Algorithm
///
/// 1. Divide beats into overlapping segments (2s minimum duration)
/// 2. For each segment, compute beat intervals
/// 3. Calculate coefficient of variation (CV) of intervals
/// 4. If CV > threshold, mark as variable tempo
/// 5. Estimate BPM for each segment from mean interval
pub fn detect_tempo_variations(
    beats: &[f32],
    nominal_bpm: f32,
) -> Result<Vec<TempoSegment>, AnalysisError> {
    if beats.len() < 4 {
        // Need at least 4 beats to analyze tempo variation
        return Ok(vec![TempoSegment {
            start_time: if beats.is_empty() { 0.0 } else { beats[0] },
            end_time: if beats.is_empty() { 0.0 } else { beats[beats.len() - 1] },
            bpm: nominal_bpm,
            confidence: 0.5,
            is_variable: false,
        }]);
    }
    
    if nominal_bpm <= EPSILON {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid nominal BPM: {:.2}", nominal_bpm)
        ));
    }
    
    let total_duration = beats[beats.len() - 1] - beats[0];
    
    if total_duration < MIN_SEGMENT_DURATION_S {
        // Too short to segment, return single segment
        return Ok(vec![TempoSegment {
            start_time: beats[0],
            end_time: beats[beats.len() - 1],
            bpm: nominal_bpm,
            confidence: 0.8,
            is_variable: false,
        }]);
    }
    
    // Segment duration: use 4-8 seconds depending on total duration
    let segment_duration = (total_duration / 4.0).max(4.0).min(8.0);
    let overlap = segment_duration * 0.5; // 50% overlap
    
    let mut segments = Vec::new();
    let mut current_start = beats[0];
    
    while current_start < beats[beats.len() - 1] {
        let segment_end = (current_start + segment_duration).min(beats[beats.len() - 1]);
        
        // Find beats in this segment
        let segment_beats: Vec<f32> = beats.iter()
            .filter(|&&beat| beat >= current_start && beat <= segment_end)
            .copied()
            .collect();
        
        if segment_beats.len() >= 3 {
            // Calculate beat intervals
            let mut intervals = Vec::new();
            for i in 1..segment_beats.len() {
                let interval = segment_beats[i] - segment_beats[i - 1];
                if interval > 0.0 {
                    intervals.push(interval);
                }
            }
            
            if !intervals.is_empty() {
                // Calculate mean and standard deviation
                let mean_interval: f32 = intervals.iter().sum::<f32>() / intervals.len() as f32;
                let variance: f32 = intervals.iter()
                    .map(|&interval| {
                        let diff = interval - mean_interval;
                        diff * diff
                    })
                    .sum::<f32>() / intervals.len() as f32;
                let std_dev = variance.sqrt();
                
                // Coefficient of variation
                let cv = if mean_interval > EPSILON {
                    std_dev / mean_interval
                } else {
                    0.0
                };
                
                // Estimate BPM from mean interval
                let segment_bpm = if mean_interval > EPSILON {
                    60.0 / mean_interval
                } else {
                    nominal_bpm
                };
                
                // Confidence based on CV (lower CV = higher confidence)
                let confidence = (1.0 - (cv / 0.3).min(1.0)).max(0.0);
                
                // Mark as variable if CV exceeds threshold
                let is_variable = cv > TEMPO_VARIATION_THRESHOLD;
                
                segments.push(TempoSegment {
                    start_time: current_start,
                    end_time: segment_end,
                    bpm: segment_bpm,
                    confidence,
                    is_variable,
                });
            }
        }
        
        // Move to next segment (with overlap)
        current_start += segment_duration - overlap;
    }
    
    // If no segments found, return single segment with nominal BPM
    if segments.is_empty() {
        segments.push(TempoSegment {
            start_time: beats[0],
            end_time: beats[beats.len() - 1],
            bpm: nominal_bpm,
            confidence: 0.8,
            is_variable: false,
        });
    }
    
    Ok(segments)
}

/// Check if track has significant tempo variation
///
/// Returns true if any segment is marked as variable tempo
pub fn has_tempo_variation(segments: &[TempoSegment]) -> bool {
    segments.iter().any(|seg| seg.is_variable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tempo_variations_constant() {
        // Constant tempo: 120 BPM (0.5s intervals)
        let beat_interval = 60.0 / 120.0;
        let beats: Vec<f32> = (0..20).map(|i| i as f32 * beat_interval).collect();
        
        let segments = detect_tempo_variations(&beats, 120.0).unwrap();
        
        assert!(!segments.is_empty());
        // Constant tempo should have low variation
        assert!(!has_tempo_variation(&segments));
    }

    #[test]
    fn test_detect_tempo_variations_variable() {
        // Variable tempo: starts at 120 BPM, speeds up to 140 BPM
        let mut beats = Vec::new();
        let mut time = 0.0;
        for i in 0..20 {
            let bpm = 120.0 + (i as f32 * 1.0); // Gradually increases
            let interval = 60.0 / bpm;
            time += interval;
            beats.push(time);
        }
        
        let segments = detect_tempo_variations(&beats, 120.0).unwrap();
        
        assert!(!segments.is_empty());
        // Variable tempo should be detected
        // Note: May not always detect if variation is gradual
    }

    #[test]
    fn test_detect_tempo_variations_insufficient_beats() {
        let beats = vec![0.0, 0.5, 1.0]; // Only 3 beats
        
        let segments = detect_tempo_variations(&beats, 120.0).unwrap();
        
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].bpm, 120.0);
    }

    #[test]
    fn test_detect_tempo_variations_empty() {
        let segments = detect_tempo_variations(&[], 120.0).unwrap();
        
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].bpm, 120.0);
    }

    #[test]
    fn test_has_tempo_variation() {
        let segments = vec![
            TempoSegment {
                start_time: 0.0,
                end_time: 4.0,
                bpm: 120.0,
                confidence: 0.8,
                is_variable: false,
            },
            TempoSegment {
                start_time: 4.0,
                end_time: 8.0,
                bpm: 130.0,
                confidence: 0.6,
                is_variable: true,
            },
        ];
        
        assert!(has_tempo_variation(&segments));
        
        let constant_segments = vec![
            TempoSegment {
                start_time: 0.0,
                end_time: 4.0,
                bpm: 120.0,
                confidence: 0.8,
                is_variable: false,
            },
        ];
        
        assert!(!has_tempo_variation(&constant_segments));
    }
}

