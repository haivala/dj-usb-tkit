//! Beat tracking modules
//!
//! Generate precise beat grid from BPM estimate:
//! - HMM Viterbi algorithm
//! - Bayesian tempo tracking
//! - Beat grid generation with downbeat detection
//!
//! # Overview
//!
//! This module provides beat tracking functionality to convert BPM estimates into
//! precise beat grids. The main entry point is `generate_beat_grid()`, which uses
//! the HMM Viterbi algorithm to find the optimal beat sequence from onset detections.
//!
//! # Algorithm Pipeline
//!
//! 1. **HMM Viterbi Tracking**: Find optimal beat sequence using Hidden Markov Model
//! 2. **Beat Grid Generation**: Convert beat positions to structured grid
//! 3. **Downbeat Detection**: Identify beat 1 of each bar (typically 4/4 time)
//! 4. **Grid Stability**: Measure consistency of beat grid
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::generate_beat_grid;
//!
//! let bpm_estimate = 120.0;
//! let bpm_confidence = 0.85;
//! let onsets = vec![0.0, 0.5, 1.0, 1.5]; // Onset times in seconds
//! let sample_rate = 44100;
//!
//! let (beat_grid, stability) = generate_beat_grid(
//!     bpm_estimate,
//!     bpm_confidence,
//!     &onsets,
//!     sample_rate,
//! )?;
//!
//! println!("Beat grid: {} beats, {} downbeats, stability={:.2}",
//!          beat_grid.beats.len(), beat_grid.downbeats.len(), stability);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

pub mod bayesian;
pub mod hmm;
pub mod tempo_variation;
pub mod time_signature;

use crate::analysis::result::BeatGrid;
use crate::error::AnalysisError;
use tempo_variation::{detect_tempo_variations, has_tempo_variation};
use time_signature::{detect_time_signature, TimeSignature};

/// Beat position in a bar
#[derive(Debug, Clone)]
pub struct BeatPosition {
    /// Beat index within bar (0, 1, 2, 3)
    pub beat_index: u32,
    
    /// Time in seconds
    pub time_seconds: f32,
    
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// Generate beat grid from BPM estimate and onsets
///
/// This is the main public API for beat tracking. It uses the HMM Viterbi algorithm
/// to find the optimal beat sequence, then generates a structured beat grid with
/// downbeats and bar boundaries.
///
/// # Arguments
///
/// * `bpm_estimate` - BPM estimate from period estimation (Phase 1B)
/// * `bpm_confidence` - Confidence in BPM estimate (0.0-1.0)
/// * `onsets` - Onset times in seconds (must be sorted)
/// * `sample_rate` - Sample rate in Hz (for logging)
///
/// # Returns
///
/// Tuple of `(BeatGrid, grid_stability)` where:
/// - `BeatGrid` contains beats, downbeats, and bar boundaries
/// - `grid_stability` is a measure of beat grid consistency (0.0-1.0)
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - BPM estimate is invalid
/// - Onsets list is empty
/// - Beat tracking fails
///
/// # Algorithm
///
/// 1. Convert onsets to sorted vector (if not already sorted)
/// 2. Run HMM Viterbi algorithm to find optimal beat sequence
/// 3. Detect tempo variations by segmenting audio and analyzing beat intervals
/// 4. If tempo variation detected, refine beats using Bayesian tracker for variable segments
/// 5. Detect time signature (4/4, 3/4, 6/8) from beat patterns
/// 6. Generate beat grid with all beats
/// 7. Detect downbeats (beat 1 of each bar, using detected time signature)
/// 8. Calculate grid stability based on beat interval consistency
///
/// # Reference
///
/// Böck, S., Krebs, F., & Schedl, M. (2016). Joint Beat and Downbeat Tracking with a
/// Recurrent Neural Network. *Proceedings of the International Society for Music
/// Information Retrieval Conference*.
pub fn generate_beat_grid(
    bpm_estimate: f32,
    bpm_confidence: f32,
    onsets: &[f32],
    sample_rate: u32,
) -> Result<(BeatGrid, f32), AnalysisError> {
    if bpm_estimate <= 0.0 || bpm_estimate > 300.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid BPM estimate: {:.2}", bpm_estimate)
        ));
    }
    
    if onsets.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Cannot generate beat grid: no onsets provided".to_string()
        ));
    }
    
    log::debug!("Generating beat grid: BPM={:.2}, confidence={:.3}, {} onsets",
                bpm_estimate, bpm_confidence, onsets.len());
    
    // Ensure onsets are sorted
    let mut sorted_onsets = onsets.to_vec();
    sorted_onsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    // Step 1: Run HMM Viterbi beat tracking (initial beat sequence)
    let tracker = hmm::HmmBeatTracker::new(bpm_estimate, sorted_onsets.clone(), sample_rate);
    let mut beat_positions = tracker.track_beats()?;
    
    if beat_positions.is_empty() {
        return Err(AnalysisError::ProcessingError(
            "HMM beat tracking produced no beats".to_string()
        ));
    }
    
    // Step 2: Detect tempo variations
    let beat_times: Vec<f32> = beat_positions.iter().map(|bp| bp.time_seconds).collect();
    let tempo_segments = detect_tempo_variations(&beat_times, bpm_estimate)?;
    let has_variation = has_tempo_variation(&tempo_segments);
    
    log::debug!("Tempo variation detection: {} segments, has_variation={}",
                tempo_segments.len(), has_variation);
    
    // Step 3: If tempo variation detected, refine beats using Bayesian tracker
    if has_variation {
        log::debug!("Tempo variation detected, refining beats with Bayesian tracker");
        
        // Use Bayesian tracker to refine beats for variable tempo segments
        let mut refined_beats = Vec::new();
        let mut bayesian_tracker = bayesian::BayesianBeatTracker::new(bpm_estimate, bpm_confidence);
        
        for segment in &tempo_segments {
            if segment.is_variable {
                // Get onsets in this segment
                let segment_onsets: Vec<f32> = sorted_onsets.iter()
                    .filter(|&&onset| onset >= segment.start_time && onset <= segment.end_time)
                    .copied()
                    .collect();
                
                if !segment_onsets.is_empty() {
                    // Update Bayesian tracker with segment onsets
                    let (updated_bpm, updated_confidence) = bayesian_tracker
                        .update_with_onsets(&segment_onsets, sample_rate)?;
                    
                    log::debug!("Segment [{:.2}s-{:.2}s]: BPM {:.2} → {:.2} (confidence: {:.3})",
                               segment.start_time, segment.end_time,
                               segment.bpm, updated_bpm, updated_confidence);
                    
                    // Re-track beats for this segment with updated BPM
                    let segment_tracker = hmm::HmmBeatTracker::new(
                        updated_bpm,
                        segment_onsets,
                        sample_rate,
                    );
                    if let Ok(segment_beats) = segment_tracker.track_beats() {
                        refined_beats.extend(segment_beats);
                    }
                }
            } else {
                // Keep original beats for constant tempo segments
                let segment_beats: Vec<BeatPosition> = beat_positions.iter()
                    .filter(|bp| bp.time_seconds >= segment.start_time && bp.time_seconds <= segment.end_time)
                    .cloned()
                    .collect();
                refined_beats.extend(segment_beats);
            }
        }
        
        // If we got refined beats, use them; otherwise keep original
        if !refined_beats.is_empty() {
            refined_beats.sort_by(|a, b| a.time_seconds.partial_cmp(&b.time_seconds).unwrap());
            beat_positions = refined_beats;
            log::debug!("Refined beats using Bayesian tracker: {} beats", beat_positions.len());
        }
    }
    
    // Step 4: Detect time signature
    let beat_times: Vec<f32> = beat_positions.iter().map(|bp| bp.time_seconds).collect();
    let (time_sig, time_sig_confidence) = detect_time_signature(&beat_times, bpm_estimate)?;
    
    log::debug!("Time signature detected: {} (confidence: {:.3})",
                time_sig.name(), time_sig_confidence);
    
    // Step 5: Generate beat grid with detected time signature
    let beat_grid = generate_beat_grid_from_positions_with_time_sig(
        &beat_positions,
        bpm_estimate,
        time_sig,
    )?;
    
    // Calculate grid stability
    let stability = calculate_grid_stability(&beat_positions, bpm_estimate)?;
    
    log::debug!("Beat grid generated: {} beats, {} downbeats, stability={:.3}, time_sig={}",
                beat_grid.beats.len(), beat_grid.downbeats.len(), stability, time_sig.name());
    
    Ok((beat_grid, stability))
}

/// Generate beat grid structure from beat positions
///
/// Converts beat positions into a structured `BeatGrid` with:
/// - All beats (sorted by time)
/// - Downbeats (beat 1 of each bar, using detected time signature)
/// - Bar boundaries (downbeat times)
///
/// # Arguments
///
/// * `beat_positions` - Beat positions from HMM tracker
/// * `bpm_estimate` - BPM estimate (for downbeat detection)
///
/// # Returns
///
/// `BeatGrid` structure with beats, downbeats, and bars
#[allow(dead_code)] // Used in tests
fn generate_beat_grid_from_positions(
    beat_positions: &[BeatPosition],
    bpm_estimate: f32,
) -> Result<BeatGrid, AnalysisError> {
    // Default to 4/4 time if not specified
    generate_beat_grid_from_positions_with_time_sig(
        beat_positions,
        bpm_estimate,
        TimeSignature::FourFour,
    )
}

/// Generate beat grid structure from beat positions with time signature
///
/// Converts beat positions into a structured `BeatGrid` with:
/// - All beats (sorted by time)
/// - Downbeats (beat 1 of each bar, using specified time signature)
/// - Bar boundaries (downbeat times)
///
/// # Arguments
///
/// * `beat_positions` - Beat positions from HMM tracker
/// * `bpm_estimate` - BPM estimate (for downbeat detection)
/// * `time_sig` - Detected time signature
///
/// # Returns
///
/// `BeatGrid` structure with beats, downbeats, and bars
fn generate_beat_grid_from_positions_with_time_sig(
    beat_positions: &[BeatPosition],
    bpm_estimate: f32,
    time_sig: TimeSignature,
) -> Result<BeatGrid, AnalysisError> {
    if beat_positions.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Cannot generate grid: no beat positions".to_string()
        ));
    }
    
    // Extract all beat times
    let mut beats: Vec<f32> = beat_positions.iter()
        .map(|bp| bp.time_seconds)
        .collect();
    
    // Sort by time (should already be sorted, but ensure it)
    beats.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    // Detect downbeats (beat 1 of each bar) using detected time signature
    let downbeats = detect_downbeats_with_time_sig(&beats, bpm_estimate, time_sig)?;
    
    // Bar boundaries are the same as downbeats (bar starts at downbeat)
    let bars = downbeats.clone();
    
    Ok(BeatGrid {
        downbeats,
        beats,
        bars,
    })
}

/// Detect downbeats (beat 1 of each bar)
///
/// Identifies downbeats by finding beats that align with the expected bar structure.
/// Uses 4/4 time signature by default.
///
/// # Arguments
///
/// * `beats` - All beat times in seconds (sorted)
/// * `bpm_estimate` - BPM estimate for calculating expected intervals
///
/// # Returns
///
/// Vector of downbeat times (beat 1 of each bar)
#[allow(dead_code)] // Used in tests
fn detect_downbeats(beats: &[f32], bpm_estimate: f32) -> Result<Vec<f32>, AnalysisError> {
    detect_downbeats_with_time_sig(beats, bpm_estimate, TimeSignature::FourFour)
}

/// Detect downbeats (beat 1 of each bar) with time signature
///
/// Identifies downbeats by finding beats that align with the expected bar structure
/// for the given time signature.
///
/// # Algorithm
///
/// 1. Calculate expected beat interval from BPM
/// 2. Calculate bar interval based on time signature (beats_per_bar × beat_interval)
/// 3. Find first beat as initial downbeat
/// 4. For each subsequent beat, check if it's approximately one bar away from last downbeat
/// 5. If yes, mark as downbeat
///
/// # Arguments
///
/// * `beats` - All beat times in seconds (sorted)
/// * `bpm_estimate` - BPM estimate for calculating expected intervals
/// * `time_sig` - Time signature (determines beats per bar)
///
/// # Returns
///
/// Vector of downbeat times (beat 1 of each bar)
fn detect_downbeats_with_time_sig(
    beats: &[f32],
    bpm_estimate: f32,
    time_sig: TimeSignature,
) -> Result<Vec<f32>, AnalysisError> {
    if beats.is_empty() {
        return Ok(Vec::new());
    }
    
    if bpm_estimate <= 0.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid BPM for downbeat detection: {:.2}", bpm_estimate)
        ));
    }
    
    let beats_per_bar = time_sig.beats_per_bar() as f32;
    let beat_interval = 60.0 / bpm_estimate;
    let bar_interval = beat_interval * beats_per_bar;
    
    // Tolerance for downbeat detection: ±10% of bar interval
    let tolerance = bar_interval * 0.1;
    
    let mut downbeats = Vec::new();
    
    // First beat is always a downbeat
    downbeats.push(beats[0]);
    
    // For each subsequent beat, check if it's approximately one bar away from last downbeat
    for &beat_time in &beats[1..] {
        let last_downbeat = downbeats[downbeats.len() - 1];
        let expected_next_downbeat = last_downbeat + bar_interval;
        let distance = (beat_time - expected_next_downbeat).abs();
        
        // If this beat is close to expected downbeat time, mark it as downbeat
        if distance <= tolerance {
            downbeats.push(beat_time);
        }
    }
    
    Ok(downbeats)
}

/// Calculate grid stability
///
/// Measures the consistency of the beat grid by computing the variance of beat intervals.
/// Lower variance = higher stability.
///
/// # Formula
///
/// stability = 1.0 / (1.0 + coefficient_of_variation)
///
/// where coefficient_of_variation = std_dev / mean
///
/// # Arguments
///
/// * `beat_positions` - Beat positions from HMM tracker
/// * `bpm_estimate` - Expected BPM (for comparison)
///
/// # Returns
///
/// Stability score (0.0-1.0), where 1.0 = perfect stability
fn calculate_grid_stability(
    beat_positions: &[BeatPosition],
    bpm_estimate: f32,
) -> Result<f32, AnalysisError> {
    if beat_positions.len() < 2 {
        // Need at least 2 beats to compute intervals
        return Ok(0.0);
    }
    
    if bpm_estimate <= 0.0 {
        return Err(AnalysisError::InvalidInput(
            format!("Invalid BPM for stability calculation: {:.2}", bpm_estimate)
        ));
    }
    
    // Calculate beat intervals
    let mut intervals = Vec::new();
    for i in 1..beat_positions.len() {
        let interval = beat_positions[i].time_seconds - beat_positions[i - 1].time_seconds;
        if interval > 0.0 {
            intervals.push(interval);
        }
    }
    
    if intervals.is_empty() {
        return Ok(0.0);
    }
    
    // Calculate mean interval
    let mean_interval: f32 = intervals.iter().sum::<f32>() / intervals.len() as f32;
    
    if mean_interval <= 1e-10 {
        return Ok(0.0);
    }
    
    // Calculate standard deviation
    let variance: f32 = intervals.iter()
        .map(|&interval| {
            let diff = interval - mean_interval;
            diff * diff
        })
        .sum::<f32>() / intervals.len() as f32;
    
    let std_dev = variance.sqrt();
    
    // Coefficient of variation
    let cv = std_dev / mean_interval;
    
    // Stability: higher when CV is lower
    // Formula: stability = 1.0 / (1.0 + cv)
    // This gives stability in range [0, 1] where:
    // - cv = 0 → stability = 1.0 (perfect)
    // - cv = 1 → stability = 0.5 (moderate)
    // - cv → ∞ → stability → 0 (unstable)
    let stability = 1.0 / (1.0 + cv);
    
    Ok(stability)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_beat_grid_basic() {
        let bpm = 120.0;
        let bpm_confidence = 0.85;
        // Create onsets at 120 BPM (0.5s intervals)
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5];
        let sample_rate = 44100;
        
        let (beat_grid, stability) = generate_beat_grid(bpm, bpm_confidence, &onsets, sample_rate).unwrap();
        
        assert!(!beat_grid.beats.is_empty());
        assert!(stability >= 0.0 && stability <= 1.0);
        
        // Beats should be sorted
        for i in 1..beat_grid.beats.len() {
            assert!(beat_grid.beats[i] > beat_grid.beats[i - 1]);
        }
    }

    #[test]
    fn test_generate_beat_grid_128bpm() {
        let bpm = 128.0;
        let bpm_confidence = 0.8;
        let beat_interval = 60.0 / 128.0;
        let onsets: Vec<f32> = (0..8).map(|i| i as f32 * beat_interval).collect();
        let sample_rate = 44100;
        
        let (beat_grid, stability) = generate_beat_grid(bpm, bpm_confidence, &onsets, sample_rate).unwrap();
        
        assert!(!beat_grid.beats.is_empty());
        assert!(stability > 0.0);
    }

    #[test]
    fn test_generate_beat_grid_invalid_bpm() {
        let onsets = vec![0.0, 0.5, 1.0];
        assert!(generate_beat_grid(0.0, 0.8, &onsets, 44100).is_err());
        assert!(generate_beat_grid(350.0, 0.8, &onsets, 44100).is_err());
    }

    #[test]
    fn test_generate_beat_grid_empty_onsets() {
        assert!(generate_beat_grid(120.0, 0.8, &[], 44100).is_err());
    }

    #[test]
    fn test_detect_downbeats() {
        // Create beats at 120 BPM (0.5s intervals, 4 beats per bar = 2s per bar)
        let beats = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0];
        let bpm = 120.0;
        
        let downbeats = detect_downbeats(&beats, bpm).unwrap();
        
        assert!(!downbeats.is_empty());
        // First beat should be downbeat
        assert_eq!(downbeats[0], 0.0);
        
        // Should detect downbeats approximately every 2 seconds (4 beats)
        // At 120 BPM: 4 beats = 2.0 seconds
        if downbeats.len() > 1 {
            let bar_interval = downbeats[1] - downbeats[0];
            assert!((bar_interval - 2.0).abs() < 0.3, "Bar interval should be ~2.0s");
        }
    }

    #[test]
    fn test_detect_downbeats_empty() {
        assert_eq!(detect_downbeats(&[], 120.0).unwrap().len(), 0);
    }

    #[test]
    fn test_detect_downbeats_single_beat() {
        let downbeats = detect_downbeats(&[0.5], 120.0).unwrap();
        assert_eq!(downbeats.len(), 1);
        assert_eq!(downbeats[0], 0.5);
    }

    #[test]
    fn test_calculate_grid_stability_perfect() {
        // Perfect 120 BPM beats (0.5s intervals)
        let beat_positions = vec![
            BeatPosition { beat_index: 0, time_seconds: 0.0, confidence: 1.0 },
            BeatPosition { beat_index: 1, time_seconds: 0.5, confidence: 1.0 },
            BeatPosition { beat_index: 2, time_seconds: 1.0, confidence: 1.0 },
            BeatPosition { beat_index: 3, time_seconds: 1.5, confidence: 1.0 },
        ];
        
        let stability = calculate_grid_stability(&beat_positions, 120.0).unwrap();
        assert!(stability > 0.9, "Perfect beats should have high stability");
    }

    #[test]
    fn test_calculate_grid_stability_variable() {
        // Variable tempo beats
        let beat_positions = vec![
            BeatPosition { beat_index: 0, time_seconds: 0.0, confidence: 1.0 },
            BeatPosition { beat_index: 1, time_seconds: 0.4, confidence: 0.8 },
            BeatPosition { beat_index: 2, time_seconds: 0.9, confidence: 0.7 },
            BeatPosition { beat_index: 3, time_seconds: 1.6, confidence: 0.6 },
        ];
        
        let stability = calculate_grid_stability(&beat_positions, 120.0).unwrap();
        assert!(stability < 0.9, "Variable tempo should have lower stability");
        assert!(stability >= 0.0 && stability <= 1.0);
    }

    #[test]
    fn test_calculate_grid_stability_insufficient_beats() {
        let beat_positions = vec![
            BeatPosition { beat_index: 0, time_seconds: 0.0, confidence: 1.0 },
        ];
        
        let stability = calculate_grid_stability(&beat_positions, 120.0).unwrap();
        assert_eq!(stability, 0.0, "Need at least 2 beats for stability calculation");
    }

    #[test]
    fn test_generate_beat_grid_from_positions() {
        let beat_positions = vec![
            BeatPosition { beat_index: 0, time_seconds: 0.0, confidence: 1.0 },
            BeatPosition { beat_index: 1, time_seconds: 0.5, confidence: 0.9 },
            BeatPosition { beat_index: 2, time_seconds: 1.0, confidence: 0.8 },
            BeatPosition { beat_index: 3, time_seconds: 1.5, confidence: 0.7 },
            BeatPosition { beat_index: 4, time_seconds: 2.0, confidence: 0.6 },
        ];
        
        let grid = generate_beat_grid_from_positions(&beat_positions, 120.0).unwrap();
        
        assert_eq!(grid.beats.len(), 5);
        assert!(!grid.downbeats.is_empty());
        assert_eq!(grid.bars.len(), grid.downbeats.len());
    }

    #[test]
    fn test_generate_beat_grid_from_positions_empty() {
        assert!(generate_beat_grid_from_positions(&[], 120.0).is_err());
    }
}

