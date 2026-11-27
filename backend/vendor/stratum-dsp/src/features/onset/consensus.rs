//! Onset consensus voting
//!
//! Combines multiple onset detection methods with weighted voting.
//!
//! Algorithm:
//! 1. Sort all onsets from 4 methods
//! 2. Cluster onsets within tolerance_ms windows (default 50ms)
//! 3. For each cluster, sum weights of methods that detected it
//! 4. Normalize confidence to [0, 1]
//! 5. Return sorted by confidence
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::onset::consensus::{vote_onsets, OnsetConsensus};
//!
//! // Note: All onsets must be in sample positions (convert frame indices if needed)
//! let consensus = OnsetConsensus {
//!     energy_flux: vec![1000, 5000],
//!     spectral_flux: vec![1050, 5100], // Converted from frame indices
//!     hfc: vec![980, 5020],            // Converted from frame indices
//!     hpss: vec![1020, 4990],          // Converted from frame indices
//! };
//!
//! let weights = [0.25, 0.25, 0.25, 0.25]; // Equal weights
//! let candidates = vote_onsets(consensus, weights, 50, 44100)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use super::OnsetCandidate;

/// Onset detection results from all methods
///
/// **Important**: All onsets must be in sample positions (not frame indices).
/// Methods that return frame indices (spectral_flux, hfc, hpss) must be converted
/// to sample positions using: `sample_position = frame_index * hop_size`
#[derive(Debug, Clone)]
pub struct OnsetConsensus {
    /// Energy flux onsets (in samples)
    pub energy_flux: Vec<usize>,
    /// Spectral flux onsets (in samples) - convert from frame indices if needed
    pub spectral_flux: Vec<usize>,
    /// HFC onsets (in samples) - convert from frame indices if needed
    pub hfc: Vec<usize>,
    /// HPSS onsets (in samples) - convert from frame indices if needed
    pub hpss: Vec<usize>,
}

/// Vote on onsets from multiple methods
///
/// This function combines onset detections from 4 different methods using
/// weighted voting. Onsets detected by multiple methods within a tolerance
/// window are clustered together, and confidence is calculated based on the
/// sum of weights from methods that detected each onset.
///
/// # Reference
///
/// Pecan, S., et al. (2017). A Comparison of Onset Detection Methods.
/// *Proceedings of the International Conference on Music Information Retrieval*.
///
/// McFee, B., & Ellis, D. P. W. (2014). Better Beat Tracking Through Robust Onset Aggregation.
/// *Proceedings of the International Society for Music Information Retrieval Conference*.
///
/// # Arguments
///
/// * `consensus` - Onset results from all methods (in samples).
///   Note: Methods that return frame indices (spectral_flux, hfc, hpss) must be
///   converted to sample positions: `sample_position = frame_index * hop_size`
/// * `weights` - Weights for each method [energy_flux, spectral_flux, hfc, hpss]
///   (should sum to 1.0 for normalized confidence)
/// * `tolerance_ms` - Time tolerance for clustering in milliseconds (default: 50ms)
/// * `sample_rate` - Sample rate in Hz
///
/// # Returns
///
/// Vector of onset candidates with confidence scores, sorted by confidence (highest first)
///
/// # Errors
///
/// Returns `AnalysisError` if parameters are invalid
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::onset::consensus::{vote_onsets, OnsetConsensus};
///
/// // Note: All onsets must be in sample positions
/// // If using spectral_flux, hfc, or hpss, convert frame indices:
/// // let spectral_onsets_samples: Vec<usize> = spectral_onsets_frames
/// //     .iter()
/// //     .map(|&frame_idx| frame_idx * hop_size)
/// //     .collect();
///
/// let consensus = OnsetConsensus {
///     energy_flux: vec![1000, 5000],
///     spectral_flux: vec![1050, 5100], // Already converted to samples
///     hfc: vec![980, 5020],            // Already converted to samples
///     hpss: vec![1020, 4990],          // Already converted to samples
/// };
///
/// let weights = [0.3, 0.3, 0.2, 0.2]; // Energy and spectral weighted higher
/// let candidates = vote_onsets(consensus, weights, 50, 44100)?;
///
/// for candidate in candidates {
///     println!("Onset at {:.3}s: confidence={:.2}, voted_by={}", 
///              candidate.time_seconds, candidate.confidence, candidate.voted_by);
/// }
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn vote_onsets(
    consensus: OnsetConsensus,
    weights: [f32; 4],
    tolerance_ms: u32,
    sample_rate: u32,
) -> Result<Vec<OnsetCandidate>, AnalysisError> {
    // Validate inputs
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
    }
    
    if tolerance_ms == 0 {
        return Err(AnalysisError::InvalidInput("Tolerance must be > 0".to_string()));
    }
    
    // Validate weights (should be non-negative, but don't require sum=1.0 for flexibility)
    for &w in &weights {
        if w < 0.0 {
            return Err(AnalysisError::InvalidInput("Weights must be non-negative".to_string()));
        }
    }
    
    log::debug!("Voting on onsets: energy_flux={}, spectral_flux={}, hfc={}, hpss={}, tolerance={}ms",
                consensus.energy_flux.len(), consensus.spectral_flux.len(),
                consensus.hfc.len(), consensus.hpss.len(), tolerance_ms);
    
    // Convert tolerance from milliseconds to samples
    let tolerance_samples = (tolerance_ms as f32 / 1000.0 * sample_rate as f32) as usize;
    
    // Step 1: Collect all onsets with their method indices
    // Method indices: 0=energy_flux, 1=spectral_flux, 2=hfc, 3=hpss
    #[derive(Debug, Clone)]
    struct OnsetWithMethod {
        sample: usize,
        method_idx: usize,
        weight: f32,
    }
    
    let mut all_onsets = Vec::new();
    
    // Add energy flux onsets
    for &sample in &consensus.energy_flux {
        all_onsets.push(OnsetWithMethod {
            sample,
            method_idx: 0,
            weight: weights[0],
        });
    }
    
    // Add spectral flux onsets
    for &sample in &consensus.spectral_flux {
        all_onsets.push(OnsetWithMethod {
            sample,
            method_idx: 1,
            weight: weights[1],
        });
    }
    
    // Add HFC onsets
    for &sample in &consensus.hfc {
        all_onsets.push(OnsetWithMethod {
            sample,
            method_idx: 2,
            weight: weights[2],
        });
    }
    
    // Add HPSS onsets
    for &sample in &consensus.hpss {
        all_onsets.push(OnsetWithMethod {
            sample,
            method_idx: 3,
            weight: weights[3],
        });
    }
    
    if all_onsets.is_empty() {
        return Ok(Vec::new());
    }
    
    // Step 2: Sort all onsets by sample position
    all_onsets.sort_by_key(|o| o.sample);
    
    // Step 3: Cluster onsets within tolerance windows
    // Use a simple greedy clustering algorithm
    // An onset is added to a cluster if it's within tolerance of ANY onset in that cluster
    let mut clusters: Vec<Vec<&OnsetWithMethod>> = Vec::new();
    
    for onset in &all_onsets {
        // Try to add to existing cluster
        let mut added = false;
        for cluster in &mut clusters {
            // Check if this onset is within tolerance of any onset in the cluster
            for existing_onset in cluster.iter() {
                if (onset.sample as i32 - existing_onset.sample as i32).abs() as usize <= tolerance_samples {
                    cluster.push(onset);
                    added = true;
                    break;
                }
            }
            if added {
                break;
            }
        }
        
        // If not added to any cluster, create a new one
        if !added {
            clusters.push(vec![onset]);
        }
    }
    
    // Step 4: For each cluster, compute confidence and create OnsetCandidate
    let mut candidates = Vec::with_capacity(clusters.len());
    
    for cluster in &clusters {
        // Calculate cluster center (average of all onsets in cluster)
        let cluster_center = cluster.iter()
            .map(|o| o.sample)
            .sum::<usize>() / cluster.len();
        
        // Sum weights from methods that detected onsets in this cluster
        let mut total_weight = 0.0;
        let mut voted_by_methods = [false; 4]; // Track which methods voted
        
        for onset in cluster {
            total_weight += onset.weight;
            voted_by_methods[onset.method_idx] = true;
        }
        
        // Count number of unique methods that voted
        let voted_by = voted_by_methods.iter().filter(|&&v| v).count() as u32;
        
        // Normalize confidence to [0, 1]
        // Maximum possible weight is sum of all weights
        let max_weight: f32 = weights.iter().sum();
        let confidence = if max_weight > 0.0 {
            (total_weight / max_weight).min(1.0).max(0.0)
        } else {
            0.0
        };
        
        // Convert sample position to seconds
        let time_seconds = cluster_center as f32 / sample_rate as f32;
        
        candidates.push(OnsetCandidate {
            time_samples: cluster_center,
            time_seconds,
            confidence,
            voted_by,
        });
    }
    
    // Step 5: Sort by confidence (highest first)
    candidates.sort_by(|a, b| {
        b.confidence.partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    
    log::debug!("Consensus voting produced {} onset candidates", candidates.len());
    
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_consensus_voting_basic() {
        // Simple test: all methods detect the same onset
        let consensus = OnsetConsensus {
            energy_flux: vec![1000],
            spectral_flux: vec![1000],
            hfc: vec![1000],
            hpss: vec![1000],
        };
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].time_samples, 1000);
        assert_eq!(candidates[0].voted_by, 4);
        assert!((candidates[0].confidence - 1.0).abs() < 0.01);
    }
    
    #[test]
    fn test_consensus_voting_clustering() {
        // Test clustering: onsets within tolerance should be merged
        let consensus = OnsetConsensus {
            energy_flux: vec![1000],
            spectral_flux: vec![1050], // Within 50ms at 44.1kHz (~2205 samples)
            hfc: vec![980],
            hpss: vec![1020],
        };
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        // 50ms tolerance at 44.1kHz = 2205 samples
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        // All should cluster into one
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].voted_by, 4);
        assert!((candidates[0].confidence - 1.0).abs() < 0.01);
    }
    
    #[test]
    fn test_consensus_voting_separate_onsets() {
        // Test: two separate onsets far apart
        let consensus = OnsetConsensus {
            energy_flux: vec![1000, 50000],
            spectral_flux: vec![1050, 50500],
            hfc: vec![980, 50200],
            hpss: vec![1020, 49900],
        };
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        // Should have 2 clusters
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].voted_by, 4);
        assert_eq!(candidates[1].voted_by, 4);
    }
    
    #[test]
    fn test_consensus_voting_partial_agreement() {
        // Test: only some methods detect an onset
        let consensus = OnsetConsensus {
            energy_flux: vec![1000],
            spectral_flux: vec![1050],
            hfc: vec![], // Doesn't detect
            hpss: vec![], // Doesn't detect
        };
        
        let weights = [0.3, 0.3, 0.2, 0.2];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].voted_by, 2);
        // Confidence should be (0.3 + 0.3) / 1.0 = 0.6
        assert!((candidates[0].confidence - 0.6).abs() < 0.01);
    }
    
    #[test]
    fn test_consensus_voting_weighted() {
        // Test: different weights affect confidence
        let consensus = OnsetConsensus {
            energy_flux: vec![1000],
            spectral_flux: vec![],
            hfc: vec![],
            hpss: vec![],
        };
        
        let weights = [0.5, 0.2, 0.2, 0.1];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].voted_by, 1);
        // Confidence should be 0.5 / 1.0 = 0.5
        assert!((candidates[0].confidence - 0.5).abs() < 0.01);
    }
    
    #[test]
    fn test_consensus_voting_empty() {
        // Test: no onsets detected
        let consensus = OnsetConsensus {
            energy_flux: vec![],
            spectral_flux: vec![],
            hfc: vec![],
            hpss: vec![],
        };
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        assert!(candidates.is_empty());
    }
    
    #[test]
    fn test_consensus_voting_sorted_by_confidence() {
        // Test: results should be sorted by confidence (highest first)
        let consensus = OnsetConsensus {
            energy_flux: vec![1000, 20000], // First: all methods
            spectral_flux: vec![1050, 20050],
            hfc: vec![980, 20100],
            hpss: vec![1020, 19950],
            // Second: only one method
        };
        
        // Add a third onset detected by 2 methods
        let mut consensus2 = consensus.clone();
        consensus2.energy_flux.push(50000);
        consensus2.spectral_flux.push(50500);
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        let candidates = vote_onsets(consensus2, weights, 50, 44100).unwrap();
        
        // Should have 3 candidates, sorted by confidence
        assert!(candidates.len() >= 2);
        
        // First should have highest confidence (all 4 methods)
        assert_eq!(candidates[0].voted_by, 4);
        
        // Confidence should be decreasing
        for i in 1..candidates.len() {
            assert!(candidates[i].confidence <= candidates[i-1].confidence);
        }
    }
    
    #[test]
    fn test_consensus_voting_invalid_parameters() {
        let consensus = OnsetConsensus {
            energy_flux: vec![1000],
            spectral_flux: vec![],
            hfc: vec![],
            hpss: vec![],
        };
        
        let weights = [0.25, 0.25, 0.25, 0.25];
        
        // Test zero sample rate
        let result = vote_onsets(consensus.clone(), weights, 50, 0);
        assert!(result.is_err());
        
        // Test zero tolerance
        let result = vote_onsets(consensus.clone(), weights, 0, 44100);
        assert!(result.is_err());
        
        // Test negative weights
        let negative_weights = [-0.1, 0.25, 0.25, 0.25];
        let result = vote_onsets(consensus, negative_weights, 50, 44100);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_consensus_voting_time_conversion() {
        // Test: verify time_seconds is calculated correctly
        let consensus = OnsetConsensus {
            energy_flux: vec![44100], // 1 second at 44.1kHz
            spectral_flux: vec![],
            hfc: vec![],
            hpss: vec![],
        };
        
        let weights = [1.0, 0.0, 0.0, 0.0];
        let candidates = vote_onsets(consensus, weights, 50, 44100).unwrap();
        
        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].time_seconds - 1.0).abs() < 0.001);
    }
}

