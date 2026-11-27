//! Confidence scoring module
//!
//! Generates trustworthiness scores for analysis results. This module provides
//! comprehensive confidence scoring that combines individual feature confidences
//! into an overall assessment of analysis quality.
//!
//! # Confidence Components
//!
//! 1. **BPM Confidence**: Based on method agreement, peak prominence, and octave error handling
//! 2. **Key Confidence**: Based on template matching score difference and key clarity
//! 3. **Grid Stability**: Based on beat grid consistency and tempo variation
//! 4. **Overall Confidence**: Weighted combination of all components
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::{analyze_audio, AnalysisConfig};
//! use stratum_dsp::analysis::confidence::compute_confidence;
//!
//! let samples = vec![0.0f32; 44100 * 30];
//! let result = analyze_audio(&samples, 44100, AnalysisConfig::default())?;
//! let confidence = compute_confidence(&result);
//!
//! println!("Overall confidence: {:.2}", confidence.overall_confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use super::result::{AnalysisResult, AnalysisFlag};
use serde::{Deserialize, Serialize};

/// Analysis confidence scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfidence {
    /// BPM confidence (0.0-1.0)
    /// 
    /// Based on:
    /// - Method agreement (autocorrelation + comb filterbank)
    /// - Peak prominence in period estimation
    /// - Octave error handling
    pub bpm_confidence: f32,
    
    /// Key confidence (0.0-1.0)
    /// 
    /// Based on:
    /// - Template matching score difference
    /// - Key clarity (tonal strength)
    /// - Chroma vector quality
    pub key_confidence: f32,
    
    /// Grid stability (0.0-1.0)
    /// 
    /// Based on:
    /// - Beat grid consistency (coefficient of variation)
    /// - Tempo variation detection
    /// - Downbeat alignment
    pub grid_stability: f32,
    
    /// Overall confidence (weighted average)
    /// 
    /// Weighted combination of BPM, key, and grid stability:
    /// - BPM: 40% weight
    /// - Key: 30% weight
    /// - Grid: 30% weight
    pub overall_confidence: f32,
    
    /// Confidence flags indicating specific issues
    pub flags: Vec<AnalysisFlag>,
}

/// Compute confidence scores for analysis result
///
/// This function analyzes the analysis result and computes comprehensive
/// confidence scores for each component (BPM, key, beat grid) as well as
/// an overall confidence score.
///
/// # Arguments
///
/// * `result` - Analysis result from `analyze_audio()`
///
/// # Returns
///
/// `AnalysisConfidence` with individual and overall confidence scores
///
/// # Algorithm
///
/// 1. **BPM Confidence**: Uses the confidence from period estimation, adjusted for:
///    - Method agreement (both autocorrelation and comb filterbank agree)
///    - Peak prominence (strong vs weak peaks)
///    - Edge cases (BPM = 0 indicates failure)
///
/// 2. **Key Confidence**: Uses the confidence from key detection, adjusted for:
///    - Key clarity (tonal strength)
///    - Template matching score difference
///    - Edge cases (low confidence indicates ambiguous/atonal music)
///
/// 3. **Grid Stability**: Uses the grid stability from beat tracking, adjusted for:
///    - Beat interval consistency
///    - Tempo variation detection
///    - Edge cases (empty grid indicates failure)
///
/// 4. **Overall Confidence**: Weighted average:
///    - BPM: 40% weight (most important for DJ use case)
///    - Key: 30% weight
///    - Grid: 30% weight
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::{analyze_audio, AnalysisConfig};
/// use stratum_dsp::analysis::confidence::compute_confidence;
///
/// let samples = vec![0.0f32; 44100 * 30];
/// let result = analyze_audio(&samples, 44100, AnalysisConfig::default())?;
/// let confidence = compute_confidence(&result);
///
/// if confidence.overall_confidence < 0.5 {
///     println!("Warning: Low confidence analysis");
/// }
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn compute_confidence(result: &AnalysisResult) -> AnalysisConfidence {
    log::debug!("Computing confidence scores for analysis result");
    
    // 1. BPM Confidence
    let bpm_confidence = compute_bpm_confidence(result);
    
    // 2. Key Confidence
    let key_confidence = compute_key_confidence(result);
    
    // 3. Grid Stability (already computed, but we validate it)
    let grid_stability = result.grid_stability.max(0.0).min(1.0);
    
    // 4. Overall Confidence (weighted average)
    // Weights: BPM=40%, Key=30%, Grid=30%
    // If any component failed (confidence = 0), reduce overall confidence
    let overall_confidence = if bpm_confidence > 0.0 && key_confidence > 0.0 {
        // All components succeeded: weighted average
        (bpm_confidence * 0.4 + key_confidence * 0.3 + grid_stability * 0.3)
            .max(0.0)
            .min(1.0)
    } else if bpm_confidence > 0.0 {
        // Only BPM succeeded: use BPM confidence with penalty
        bpm_confidence * 0.6
    } else if key_confidence > 0.0 {
        // Only key succeeded: use key confidence with penalty
        key_confidence * 0.6
    } else {
        // All components failed
        0.0
    };
    
    // 5. Collect flags
    let mut flags = result.metadata.flags.clone();
    
    // Add confidence-based flags
    if bpm_confidence < 0.3 {
        flags.push(AnalysisFlag::MultimodalBpm);
    }
    if key_confidence < 0.2 {
        flags.push(AnalysisFlag::WeakTonality);
    }
    if grid_stability < 0.3 {
        flags.push(AnalysisFlag::TempoVariation);
    }
    
    log::debug!(
        "Confidence scores: BPM={:.3}, Key={:.3}, Grid={:.3}, Overall={:.3}",
        bpm_confidence,
        key_confidence,
        grid_stability,
        overall_confidence
    );
    
    AnalysisConfidence {
        bpm_confidence,
        key_confidence,
        grid_stability,
        overall_confidence,
        flags,
    }
}

impl AnalysisConfidence {
    /// Check if overall confidence is high (>= 0.7)
    ///
    /// # Returns
    ///
    /// `true` if overall confidence is high, `false` otherwise
    pub fn is_high_confidence(&self) -> bool {
        self.overall_confidence >= 0.7
    }
    
    /// Check if overall confidence is low (< 0.5)
    ///
    /// # Returns
    ///
    /// `true` if overall confidence is low, `false` otherwise
    pub fn is_low_confidence(&self) -> bool {
        self.overall_confidence < 0.5
    }
    
    /// Check if overall confidence is medium (0.5-0.7)
    ///
    /// # Returns
    ///
    /// `true` if overall confidence is medium, `false` otherwise
    pub fn is_medium_confidence(&self) -> bool {
        self.overall_confidence >= 0.5 && self.overall_confidence < 0.7
    }
    
    /// Get a human-readable confidence level description
    ///
    /// # Returns
    ///
    /// String describing the confidence level: "High", "Medium", or "Low"
    pub fn confidence_level(&self) -> &'static str {
        if self.is_high_confidence() {
            "High"
        } else if self.is_low_confidence() {
            "Low"
        } else {
            "Medium"
        }
    }
}

/// Compute BPM confidence from analysis result
///
/// BPM confidence is based on:
/// - The confidence score from period estimation
/// - Method agreement (both autocorrelation and comb filterbank agree)
/// - Edge cases (BPM = 0 indicates failure)
fn compute_bpm_confidence(result: &AnalysisResult) -> f32 {
    if result.bpm <= 0.0 {
        // BPM detection failed
        return 0.0;
    }
    
    // Use the confidence from period estimation
    // This already includes method agreement and peak prominence
    let base_confidence = result.bpm_confidence.max(0.0).min(1.0);
    
    // Additional adjustments based on metadata
    // Check if there are warnings about BPM
    let has_bpm_warning = result.metadata.confidence_warnings.iter()
        .any(|w| w.contains("BPM"));
    
    if has_bpm_warning {
        // Reduce confidence if there are warnings
        base_confidence * 0.7
    } else {
        base_confidence
    }
}

/// Compute key confidence from analysis result
///
/// Key confidence is based on:
/// - The confidence score from key detection
/// - Key clarity (tonal strength) - directly incorporated
/// - Edge cases (low confidence indicates ambiguous/atonal music)
fn compute_key_confidence(result: &AnalysisResult) -> f32 {
    if result.key_confidence <= 0.0 {
        // Key detection failed or returned default
        return 0.0;
    }
    
    // Use the confidence from key detection
    // This already includes template matching score difference
    let base_confidence = result.key_confidence.max(0.0).min(1.0);
    
    // Incorporate key clarity directly: low clarity reduces confidence
    // Key clarity is a strong indicator of detection reliability
    let clarity_adjustment = if result.key_clarity < 0.2 {
        // Low clarity: significant penalty
        0.6
    } else if result.key_clarity < 0.5 {
        // Medium clarity: moderate penalty
        0.85
    } else {
        // High clarity: no penalty (or slight boost)
        1.0
    };
    
    // Additional adjustments based on metadata warnings
    let has_key_warning = result.metadata.confidence_warnings.iter()
        .any(|w| w.contains("key") || w.contains("Key") || w.contains("tonality"));
    
    let warning_adjustment = if has_key_warning {
        0.7
    } else {
        1.0
    };
    
    // Apply both adjustments (multiplicative)
    base_confidence * clarity_adjustment * warning_adjustment
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::result::{AnalysisResult, AnalysisMetadata, BeatGrid, Key};
    
    fn create_test_result(
        bpm: f32,
        bpm_confidence: f32,
        key: Key,
        key_confidence: f32,
        key_clarity: f32,
        grid_stability: f32,
    ) -> AnalysisResult {
        AnalysisResult {
            bpm,
            bpm_confidence,
            key,
            key_confidence,
            key_clarity,
            beat_grid: BeatGrid {
                downbeats: vec![],
                beats: vec![],
                bars: vec![],
            },
            grid_stability,
            metadata: AnalysisMetadata {
                duration_seconds: 30.0,
                sample_rate: 44100,
                processing_time_ms: 100.0,
                algorithm_version: "0.1.0-alpha".to_string(),
                onset_method_consensus: 1.0,
                methods_used: vec![],
                flags: vec![],
                confidence_warnings: vec![],
                tempogram_candidates: None,
                tempogram_multi_res_triggered: None,
                tempogram_multi_res_used: None,
                tempogram_percussive_triggered: None,
                tempogram_percussive_used: None,
            },
        }
    }
    
    #[test]
    fn test_compute_confidence_all_good() {
        let result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.8,
            0.7, // High key clarity
            0.85,
        );
        
        let confidence = compute_confidence(&result);
        
        assert_eq!(confidence.bpm_confidence, 0.9);
        assert_eq!(confidence.key_confidence, 0.8);
        assert_eq!(confidence.grid_stability, 0.85);
        
        // Overall: 0.9*0.4 + 0.8*0.3 + 0.85*0.3 = 0.36 + 0.24 + 0.255 = 0.855
        assert!((confidence.overall_confidence - 0.855).abs() < 0.01);
    }
    
    #[test]
    fn test_compute_confidence_bpm_failed() {
        let result = create_test_result(
            0.0,
            0.0,
            Key::Major(0),
            0.8,
            0.7, // High key clarity
            0.85,
        );
        
        let confidence = compute_confidence(&result);
        
        assert_eq!(confidence.bpm_confidence, 0.0);
        assert_eq!(confidence.key_confidence, 0.8);
        assert_eq!(confidence.grid_stability, 0.85);
        
        // Overall: Only key and grid, but BPM failed so overall is reduced
        // Only key succeeded: 0.8 * 0.6 = 0.48
        assert!((confidence.overall_confidence - 0.48).abs() < 0.01);
    }
    
    #[test]
    fn test_compute_confidence_key_failed() {
        let result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.0,
            0.0, // No key clarity when key failed
            0.85,
        );
        
        let confidence = compute_confidence(&result);
        
        assert_eq!(confidence.bpm_confidence, 0.9);
        assert_eq!(confidence.key_confidence, 0.0);
        assert_eq!(confidence.grid_stability, 0.85);
        
        // Overall: Only BPM succeeded: 0.9 * 0.6 = 0.54
        assert!((confidence.overall_confidence - 0.54).abs() < 0.01);
    }
    
    #[test]
    fn test_compute_confidence_all_failed() {
        let result = create_test_result(
            0.0,
            0.0,
            Key::Major(0),
            0.0,
            0.0, // No key clarity
            0.0,
        );
        
        let confidence = compute_confidence(&result);
        
        assert_eq!(confidence.bpm_confidence, 0.0);
        assert_eq!(confidence.key_confidence, 0.0);
        assert_eq!(confidence.grid_stability, 0.0);
        assert_eq!(confidence.overall_confidence, 0.0);
    }
    
    #[test]
    fn test_compute_confidence_with_warnings() {
        let mut result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.8,
            0.7, // High key clarity
            0.85,
        );
        
        result.metadata.confidence_warnings.push(
            "BPM detection failed: insufficient onsets".to_string()
        );
        
        let confidence = compute_confidence(&result);
        
        // BPM confidence should be reduced due to warning
        assert!(confidence.bpm_confidence < 0.9);
        assert!(confidence.bpm_confidence > 0.0);
    }
    
    #[test]
    fn test_compute_confidence_clamping() {
        // Test that confidences are clamped to [0, 1]
        let result = create_test_result(
            120.0,
            1.5, // > 1.0
            Key::Major(0),
            -0.5, // < 0.0
            0.7, // Normal key clarity
            2.0, // > 1.0
        );
        
        let confidence = compute_confidence(&result);
        
        assert!(confidence.bpm_confidence <= 1.0);
        assert!(confidence.key_confidence >= 0.0);
        assert!(confidence.grid_stability <= 1.0);
        assert!(confidence.overall_confidence >= 0.0);
        assert!(confidence.overall_confidence <= 1.0);
    }
    
    #[test]
    fn test_confidence_helper_methods() {
        let result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.8,
            0.7,
            0.85,
        );
        
        let confidence = compute_confidence(&result);
        
        // Should be high confidence
        assert!(confidence.is_high_confidence());
        assert!(!confidence.is_low_confidence());
        assert!(!confidence.is_medium_confidence());
        assert_eq!(confidence.confidence_level(), "High");
        
        // Test low confidence case
        let low_result = create_test_result(
            0.0,
            0.0,
            Key::Major(0),
            0.0,
            0.0,
            0.0,
        );
        
        let low_confidence = compute_confidence(&low_result);
        assert!(low_confidence.is_low_confidence());
        assert!(!low_confidence.is_high_confidence());
        assert_eq!(low_confidence.confidence_level(), "Low");
    }
    
    #[test]
    fn test_key_clarity_adjustment() {
        // Test that low key clarity reduces confidence
        let high_clarity_result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.8,
            0.7, // High clarity
            0.85,
        );
        
        let low_clarity_result = create_test_result(
            120.0,
            0.9,
            Key::Major(0),
            0.8,
            0.1, // Low clarity
            0.85,
        );
        
        let high_conf = compute_confidence(&high_clarity_result);
        let low_conf = compute_confidence(&low_clarity_result);
        
        // Low clarity should result in lower key confidence
        assert!(low_conf.key_confidence < high_conf.key_confidence);
    }
}

