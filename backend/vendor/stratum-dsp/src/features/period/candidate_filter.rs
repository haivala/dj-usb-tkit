//! BPM candidate filtering and merging
//!
//! Merges results from autocorrelation and comb filter, handles octave errors.
//!
//! # Algorithm
//!
//! This module merges BPM candidates from multiple estimation methods
//! (autocorrelation and comb filterbank) and handles common issues:
//!
//! 1. **Octave Errors**: Autocorrelation often detects 2x or 0.5x the true BPM
//!    - Detect when candidates are related by octave (2x or 0.5x)
//!    - Prefer candidates that both methods agree on
//!
//! 2. **Candidate Merging**: Combine results from both methods
//!    - Group candidates within tolerance (e.g., ±2 BPM)
//!    - Boost confidence when both methods agree
//!    - Track method agreement count
//!
//! 3. **Final Selection**: Return best candidates with confidence scores
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::candidate_filter::merge_bpm_candidates;
//! use stratum_dsp::features::period::{BpmCandidate, BpmEstimate};
//!
//! let autocorr = vec![
//!     BpmCandidate { bpm: 120.0, confidence: 0.9 },
//!     BpmCandidate { bpm: 60.0, confidence: 0.7 }, // Octave error
//! ];
//! let comb = vec![
//!     BpmCandidate { bpm: 120.0, confidence: 0.85 },
//! ];
//! let merged = merge_bpm_candidates(autocorr, comb, 50.0)?;
//! // Returns BpmEstimate with bpm=120.0, confidence boosted, method_agreement=2
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use super::{BpmCandidate, BpmEstimate};

// EPSILON not currently used, but kept for consistency with other modules
const DEFAULT_BPM_TOLERANCE: f32 = 2.0; // ±2 BPM for grouping
const CONSENSUS_TOLERANCE: f32 = 2.5; // ±2.5 BPM for consensus detection

/// Boost confidence for candidates that both methods agree on
///
/// This function identifies candidates that appear in both autocorrelation
/// and comb filter top 5 lists (within tolerance) and boosts their confidence.
/// Also checks for harmonic relationships (2x, 0.5x, etc.) to catch cases where
/// one method finds a harmonic of the correct answer.
fn boost_consensus_candidates(
    autocorr_top5: &[BpmCandidate],
    comb_top5: &[BpmCandidate],
    estimates: &mut [BpmEstimate],
) {
    for estimate in estimates.iter_mut() {
        // Check if this estimate is close to any autocorr top 5 candidate (direct match)
        let autocorr_direct_match = autocorr_top5.iter().any(|a| {
            (a.bpm - estimate.bpm).abs() < CONSENSUS_TOLERANCE
        });
        
        // Check if this estimate is close to any comb top 5 candidate (direct match)
        let comb_direct_match = comb_top5.iter().any(|c| {
            (c.bpm - estimate.bpm).abs() < CONSENSUS_TOLERANCE
        });
        
        // Check for harmonic relationships (2x, 0.5x, 1.5x, 0.75x)
        let autocorr_harmonic_match = autocorr_top5.iter().any(|a| {
            let ratio = (a.bpm / estimate.bpm).max(estimate.bpm / a.bpm);
            (ratio - 2.0).abs() < 0.1 || (ratio - 1.5).abs() < 0.1 || (ratio - 0.75).abs() < 0.1
        });
        
        let comb_harmonic_match = comb_top5.iter().any(|c| {
            let ratio = (c.bpm / estimate.bpm).max(estimate.bpm / c.bpm);
            (ratio - 2.0).abs() < 0.1 || (ratio - 1.5).abs() < 0.1 || (ratio - 0.75).abs() < 0.1
        });
        
        // If both methods agree (direct match), strong boost
        if autocorr_direct_match && comb_direct_match {
            estimate.confidence *= 1.5; // 50% boost for direct consensus
            log::debug!(
                "Consensus boost (direct): {:.2} BPM (confidence: {:.3} → {:.3})",
                estimate.bpm,
                estimate.confidence / 1.5,
                estimate.confidence
            );
        }
        // If one method has direct match and other has harmonic, moderate boost
        else if (autocorr_direct_match && comb_harmonic_match) || (comb_direct_match && autocorr_harmonic_match) {
            estimate.confidence *= 1.3; // 30% boost for harmonic consensus
            log::debug!(
                "Consensus boost (harmonic): {:.2} BPM (confidence: {:.3} → {:.3})",
                estimate.bpm,
                estimate.confidence / 1.3,
                estimate.confidence
            );
        }
        // If estimate is in comb top 5 and in reasonable range (60-180), strong boost
        // This helps when comb filter finds the right answer but autocorr doesn't
        if comb_direct_match && estimate.bpm >= 60.0 && estimate.bpm <= 180.0 {
            estimate.confidence *= 1.4; // 40% boost for reasonable comb filter candidate
            log::debug!(
                "Consensus boost (reasonable range): {:.2} BPM (confidence: {:.3} → {:.3})",
                estimate.bpm,
                estimate.confidence / 1.4,
                estimate.confidence
            );
        }
    }
}

/// Merge BPM candidates from multiple methods
///
/// Combines results from autocorrelation and comb filterbank, handling
/// octave errors and boosting confidence when methods agree.
///
/// # Arguments
///
/// * `autocorr` - Candidates from autocorrelation (sorted by confidence)
/// * `comb` - Candidates from comb filterbank (sorted by confidence)
/// * `octave_tolerance_cents` - Tolerance for octave detection (default: 50 cents ≈ 2.9%)
///
/// # Returns
///
/// Merged BPM estimates with method agreement, sorted by confidence (highest first)
///
/// # Errors
///
/// Returns `AnalysisError` if numerical errors occur during merging
///
/// # Algorithm Details
///
/// 1. **Octave Detection**: Check if candidates are related by 2x or 0.5x
///    - If autocorr finds 60 BPM and comb finds 120 BPM, prefer 120 BPM
///    - If autocorr finds 240 BPM and comb finds 120 BPM, prefer 120 BPM
///
/// 2. **Candidate Grouping**: Group candidates within ±2 BPM tolerance
///    - Average BPM within group
///    - Sum confidence scores
///    - Count method agreement
///
/// 3. **Confidence Boosting**: Boost confidence when both methods agree
///    - Single method: confidence = original
///    - Both methods: confidence = min(1.0, (conf1 + conf2) * 1.2)
pub fn merge_bpm_candidates(
    autocorr: Vec<BpmCandidate>,
    comb: Vec<BpmCandidate>,
    octave_tolerance_cents: f32,
) -> Result<Vec<BpmEstimate>, crate::error::AnalysisError> {
    log::debug!(
        "Merging BPM candidates: {} autocorr, {} comb, octave_tolerance={:.1} cents",
        autocorr.len(),
        comb.len(),
        octave_tolerance_cents
    );

    if autocorr.is_empty() && comb.is_empty() {
        return Ok(vec![]);
    }

    // Convert octave tolerance from cents to BPM ratio
    // 50 cents ≈ 2.9% ≈ 3.5 BPM at 120 BPM
    let octave_tolerance_ratio = (octave_tolerance_cents / 1200.0).exp2();

    // Step 1: Handle octave errors (conservative approach)
    // Only correct if the target value (from comb) is ALSO in the top candidates
    // This prevents correcting to subharmonics that comb filter found by accident
    let mut autocorr_corrected = autocorr;
    let comb_corrected = comb;
    
    // Only consider comb filter top 3 for octave correction (most confident)
    let comb_top3: Vec<&BpmCandidate> = comb_corrected.iter().take(3).collect();

    // Check for 2x octave errors (autocorr finds 2x BPM)
    // Use multiplicative tolerance: ratio should be close to 2.0 within tolerance
    // Only correct if comb candidate is in top 3 (high confidence)
    for autocorr_candidate in &mut autocorr_corrected {
        for comb_candidate in &comb_top3 {
            let ratio = autocorr_candidate.bpm / comb_candidate.bpm;
            // Check if ratio is close to 2.0 (within multiplicative tolerance)
            let ratio_to_target = ratio / 2.0;
            if (ratio_to_target - 1.0).abs() < (octave_tolerance_ratio - 1.0) {
                // Only correct if comb candidate is in reasonable range OR autocorr is way out
                // This prevents correcting 161 BPM → 40 BPM when 40 is a subharmonic
                let should_correct = (comb_candidate.bpm >= 60.0 && comb_candidate.bpm <= 180.0) ||
                                     (autocorr_candidate.bpm > 200.0 || autocorr_candidate.bpm < 30.0);
                
                if should_correct {
                    log::debug!(
                        "Octave error detected (2x): autocorr={:.2}, comb={:.2}, ratio={:.3}, correcting to {:.2}",
                        autocorr_candidate.bpm,
                        comb_candidate.bpm,
                        ratio,
                        comb_candidate.bpm
                    );
                    autocorr_candidate.bpm = comb_candidate.bpm;
                    break;
                }
            }
        }
    }

    // Check for 0.5x octave errors (autocorr finds 0.5x BPM)
    // Only correct if comb candidate is in top 3 and reasonable
    for autocorr_candidate in &mut autocorr_corrected {
        for comb_candidate in &comb_top3 {
            let ratio = comb_candidate.bpm / autocorr_candidate.bpm;
            // Check if ratio is close to 2.0 (within multiplicative tolerance)
            let ratio_to_target = ratio / 2.0;
            if (ratio_to_target - 1.0).abs() < (octave_tolerance_ratio - 1.0) {
                // Only correct if comb candidate is in reasonable range
                // This prevents correcting to subharmonics
                if comb_candidate.bpm >= 60.0 && comb_candidate.bpm <= 180.0 {
                    log::debug!(
                        "Octave error detected (0.5x): autocorr={:.2}, comb={:.2}, ratio={:.3}, correcting to {:.2}",
                        autocorr_candidate.bpm,
                        comb_candidate.bpm,
                        ratio,
                        comb_candidate.bpm
                    );
                    autocorr_candidate.bpm = comb_candidate.bpm;
                    break;
                }
            }
        }
    }

    // Step 2: Check for method disagreement (before limiting)
    let autocorr_top = autocorr_corrected.first();
    let comb_top = comb_corrected.first();
    let has_significant_disagreement = if let (Some(ac), Some(cc)) = (autocorr_top, comb_top) {
        let bpm_diff = (ac.bpm - cc.bpm).abs();
        // If methods disagree by more than 10 BPM and neither is an octave error, flag as uncertain
        bpm_diff > 10.0 && bpm_diff < 50.0 // Not an octave, but significant disagreement
    } else {
        false
    };

    // Step 4: Limit candidates to top N from each method to avoid noise
    // Only consider top 10 from each method for better quality
    // BUT: Also preserve any reasonable-range candidates (60-180) even if not in top 10
    const MAX_CANDIDATES_PER_METHOD: usize = 10;
    let mut autocorr_limited: Vec<BpmCandidate> = autocorr_corrected
        .iter()
        .take(MAX_CANDIDATES_PER_METHOD)
        .cloned()
        .collect();
    
    // Add any reasonable-range candidates that weren't in top 10
    let mut added_reasonable = 0;
    for candidate in &autocorr_corrected {
        if candidate.bpm >= 60.0 && candidate.bpm <= 180.0 {
            if !autocorr_limited.iter().any(|c| (c.bpm - candidate.bpm).abs() < 1.0) {
                autocorr_limited.push(candidate.clone());
                added_reasonable += 1;
            }
        }
    }
    if added_reasonable > 0 {
        log::debug!(
            "Added {} reasonable-range autocorrelation candidates (60-180 BPM) to limited list",
            added_reasonable
        );
    }
    
    let comb_limited: Vec<BpmCandidate> = comb_corrected
        .into_iter()
        .take(MAX_CANDIDATES_PER_METHOD)
        .collect();

    // Step 5: Group candidates within tolerance
    let mut groups: Vec<(f32, f32, u32, f32)> = Vec::new(); // (bpm, total_confidence, method_count, max_confidence)

    // Process autocorr candidates
    for candidate in &autocorr_limited {
        let mut found_group = false;
        for group in &mut groups {
            if (candidate.bpm - group.0).abs() <= DEFAULT_BPM_TOLERANCE {
                // Add to existing group
                let count = group.2;
                group.0 = (group.0 * (count as f32) + candidate.bpm) / ((count + 1) as f32);
                group.1 += candidate.confidence;
                group.2 += 1;
                group.3 = group.3.max(candidate.confidence); // Track max confidence in group
                found_group = true;
                break;
            }
        }
        if !found_group {
            groups.push((candidate.bpm, candidate.confidence, 1, candidate.confidence));
        }
    }

    // Process comb candidates
    for candidate in &comb_limited {
        let mut found_group = false;
        for group in &mut groups {
            if (candidate.bpm - group.0).abs() <= DEFAULT_BPM_TOLERANCE {
                // Add to existing group
                let count = group.2;
                group.0 = (group.0 * (count as f32) + candidate.bpm) / ((count + 1) as f32);
                group.1 += candidate.confidence;
                group.2 += 1;
                group.3 = group.3.max(candidate.confidence); // Track max confidence in group
                found_group = true;
                break;
            }
        }
        if !found_group {
            groups.push((candidate.bpm, candidate.confidence, 1, candidate.confidence));
        }
    }

    // Step 5: Convert groups to BpmEstimate with confidence boosting
    let mut estimates: Vec<BpmEstimate> = groups
        .into_iter()
        .map(|(bpm, total_conf, method_count, max_conf)| {
            // Boost confidence when both methods agree
            let mut confidence = if method_count >= 2 {
                // Both methods agree: use average of total confidence and max confidence
                // This prevents low-quality candidates from dominating
                let avg_conf = total_conf / method_count as f32;
                ((avg_conf + max_conf) / 2.0 * 1.2).min(1.0)
            } else {
                // Single method: use original confidence
                total_conf.min(1.0)
            };

            // Penalize confidence if methods significantly disagree
            if has_significant_disagreement && method_count == 1 {
                // Single method result when methods disagree: reduce confidence
                confidence *= 0.7;
            }

            BpmEstimate {
                bpm,
                confidence,
                method_agreement: method_count,
            }
        })
        .collect();

    // Step 6: Boost candidates that both methods agree on (consensus boost)
    let autocorr_top5: Vec<BpmCandidate> = autocorr_limited
        .iter()
        .take(5)
        .cloned()
        .collect();
    let comb_top5: Vec<BpmCandidate> = comb_limited
        .iter()
        .take(5)
        .cloned()
        .collect();
    
    boost_consensus_candidates(&autocorr_top5, &comb_top5, &mut estimates);

    // Also check if reasonable candidates exist but aren't in top 5
    let has_reasonable_in_top5 = estimates.iter().take(5).any(|e| e.bpm >= 60.0 && e.bpm <= 180.0);
    log::debug!(
        "Safety check: {} reasonable candidates in top 5, {} total reasonable candidates in merged list",
        estimates.iter().take(5).filter(|e| e.bpm >= 60.0 && e.bpm <= 180.0).count(),
        estimates.iter().filter(|e| e.bpm >= 60.0 && e.bpm <= 180.0).count()
    );
    if !has_reasonable_in_top5 {
        // Find the best reasonable-range candidate (highest confidence in 60-180 range)
        if let Some(best_reasonable_idx) = estimates.iter()
            .position(|e| e.bpm >= 60.0 && e.bpm <= 180.0) {
            let best_reasonable = &mut estimates[best_reasonable_idx];
            log::debug!(
                "Safety boost: No reasonable-range candidate in top 5, boosting {:.2} BPM (was at position {}, confidence: {:.3})",
                best_reasonable.bpm,
                best_reasonable_idx + 1,
                best_reasonable.confidence
            );
            best_reasonable.confidence *= 2.0; // Very strong boost
        } else {
            log::warn!("No reasonable-range candidates (60-180 BPM) found in merged estimates! Total estimates: {}", estimates.len());
        }
    }

    // Sort by confidence (highest first), but prefer method agreement when confidence is similar
    // Also prefer candidates in reasonable BPM range (60-180) over very low/high values
    estimates.sort_by(|a, b| {
        // Prefer reasonable range (60-180 BPM) when confidence is close
        let a_in_range = a.bpm >= 60.0 && a.bpm <= 180.0;
        let b_in_range = b.bpm >= 60.0 && b.bpm <= 180.0;
        
        // Strongly prefer reasonable range (60-180) when confidence is within 30%
        // This prevents subharmonics and very high BPM from winning
        // Also penalize out-of-range candidates by reducing their effective confidence
        // Use aggressive penalty: 0.5x for out-of-range (halves their confidence)
        let a_effective_conf = if !a_in_range { a.confidence * 0.5 } else { a.confidence };
        let b_effective_conf = if !b_in_range { b.confidence * 0.5 } else { b.confidence };
        
        let effective_conf_cmp = b_effective_conf
            .partial_cmp(&a_effective_conf)
            .unwrap_or(std::cmp::Ordering::Equal);
        
        // If effective confidence is close (within 50%), strongly prefer in-range
        // This ensures reasonable candidates win when available, even if slightly lower confidence
        if (a_effective_conf - b_effective_conf).abs() < 0.5 {
            match (a_in_range, b_in_range) {
                (true, false) => return std::cmp::Ordering::Less, // a is better (in range)
                (false, true) => return std::cmp::Ordering::Greater, // b is better (in range)
                _ => {} // Both in or both out, use effective confidence
            }
        }
        
        // Use effective confidence for primary sort
        if effective_conf_cmp != std::cmp::Ordering::Equal {
            return effective_conf_cmp;
        }
        
        // Secondary sort: method agreement (when effective confidence is within 10%)
        if effective_conf_cmp == std::cmp::Ordering::Equal || (a_effective_conf - b_effective_conf).abs() < 0.1 {
            b.method_agreement.cmp(&a.method_agreement)
        } else {
            effective_conf_cmp
        }
    });

    log::debug!(
        "Merged to {} BPM estimates (best: {:.2} BPM, confidence: {:.3}, agreement: {})",
        estimates.len(),
        estimates.first().map(|e| e.bpm).unwrap_or(0.0),
        estimates.first().map(|e| e.confidence).unwrap_or(0.0),
        estimates.first().map(|e| e.method_agreement).unwrap_or(0)
    );

    Ok(estimates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_candidates_agreement() {
        // Both methods agree on 120 BPM
        let autocorr = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.9,
            },
        ];
        let comb = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.85,
            },
        ];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        assert!(!merged.is_empty());
        let best = &merged[0];
        assert!((best.bpm - 120.0).abs() < 1.0);
        assert!(best.confidence > 0.9, "Confidence should be boosted");
        assert_eq!(best.method_agreement, 2);
    }

    #[test]
    fn test_merge_candidates_octave_error() {
        // Autocorr finds 2x (octave error), comb finds correct
        let autocorr = vec![
            BpmCandidate {
                bpm: 240.0, // 2x error
                confidence: 0.8,
            },
        ];
        let comb = vec![
            BpmCandidate {
                bpm: 120.0, // Correct
                confidence: 0.9,
            },
        ];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        assert!(!merged.is_empty());
        let best = &merged[0];
        // Should prefer comb's value (120 BPM)
        assert!((best.bpm - 120.0).abs() < 1.0);
    }

    #[test]
    fn test_merge_candidates_octave_error_half() {
        // Autocorr finds 0.5x (octave error), comb finds correct
        let autocorr = vec![
            BpmCandidate {
                bpm: 60.0, // 0.5x error
                confidence: 0.8,
            },
        ];
        let comb = vec![
            BpmCandidate {
                bpm: 120.0, // Correct
                confidence: 0.9,
            },
        ];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        assert!(!merged.is_empty());
        let best = &merged[0];
        // Should prefer comb's value (120 BPM)
        assert!((best.bpm - 120.0).abs() < 1.0);
    }

    #[test]
    fn test_merge_candidates_grouping() {
        // Candidates within tolerance should be grouped
        let autocorr = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.8,
            },
            BpmCandidate {
                bpm: 121.0, // Within ±2 BPM
                confidence: 0.7,
            },
        ];
        let comb = vec![
            BpmCandidate {
                bpm: 120.5, // Within ±2 BPM
                confidence: 0.85,
            },
        ];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        // Should group into one estimate
        assert_eq!(merged.len(), 1);
        let best = &merged[0];
        // Average should be around 120.2
        assert!((best.bpm - 120.0).abs() < 2.0);
        assert_eq!(best.method_agreement, 3); // All 3 candidates grouped
    }

    #[test]
    fn test_merge_candidates_empty() {
        let merged = merge_bpm_candidates(vec![], vec![], 50.0).unwrap();
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_candidates_single_method() {
        // Only autocorr has candidates
        let autocorr = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.8,
            },
        ];
        let comb = vec![];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        assert!(!merged.is_empty());
        let best = &merged[0];
        assert_eq!(best.method_agreement, 1);
        // Confidence should not be boosted (only one method)
        assert!(best.confidence <= 0.8);
    }

    #[test]
    fn test_merge_candidates_sorted() {
        let autocorr = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.9,
            },
            BpmCandidate {
                bpm: 130.0,
                confidence: 0.7,
            },
        ];
        let comb = vec![
            BpmCandidate {
                bpm: 120.0,
                confidence: 0.85,
            },
        ];

        let merged = merge_bpm_candidates(autocorr, comb, 50.0).unwrap();

        // Should be sorted by confidence (highest first)
        for i in 1..merged.len() {
            assert!(
                merged[i - 1].confidence >= merged[i].confidence,
                "Should be sorted by confidence"
            );
        }
    }
}

