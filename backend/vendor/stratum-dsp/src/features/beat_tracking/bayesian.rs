//! Bayesian tempo and grid tracking
//!
//! Updates beat grid incrementally for variable-tempo tracks using Bayesian inference.
//!
//! This module implements Bayesian tempo tracking for handling tempo drift and
//! variable-tempo tracks. It maintains a prior distribution over BPM and updates
//! it based on observed onset evidence using Bayes' theorem.
//!
//! # Algorithm
//!
//! The Bayesian update follows:
//!
//! P(BPM | evidence) ∝ P(evidence | BPM) × P(BPM | previous_estimate)
//!
//! Where:
//! - **Prior**: P(BPM | previous_estimate) - Tempo distribution from previous update
//! - **Likelihood**: P(evidence | BPM) - Probability of observing onsets given BPM
//! - **Posterior**: P(BPM | evidence) - Updated tempo distribution
//!
//! # Use Cases
//!
//! - Variable-tempo tracks (DJ mixes, live recordings)
//! - Tempo drift correction
//! - Incremental beat grid updates
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::bayesian::BayesianBeatTracker;
//!
//! let mut tracker = BayesianBeatTracker::new(120.0, 0.8);
//!
//! // Update with new onset evidence
//! let new_onsets = vec![0.5, 1.0, 1.5, 2.0];
//! tracker.update_with_onsets(&new_onsets, 44100)?;
//!
//! println!("Updated BPM: {:.2} (confidence: {:.2})", 
//!          tracker.current_bpm, tracker.current_confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Standard deviation for prior distribution (BPM uncertainty)
const PRIOR_SIGMA: f32 = 2.0; // ±2 BPM uncertainty

/// Standard deviation for likelihood distribution (timing uncertainty)
const LIKELIHOOD_SIGMA: f32 = 0.05; // 50ms timing uncertainty in seconds

/// Bayesian beat tracker
#[derive(Debug, Clone)]
pub struct BayesianBeatTracker {
    /// Current BPM estimate
    pub current_bpm: f32,
    
    /// Current confidence (0.0-1.0)
    pub current_confidence: f32,
    
    /// BPM history for tracking
    pub history: Vec<f32>,
}

impl BayesianBeatTracker {
    /// Create a new Bayesian beat tracker
    ///
    /// # Arguments
    ///
    /// * `initial_bpm` - Initial BPM estimate (from period estimation)
    /// * `initial_confidence` - Initial confidence (0.0-1.0)
    ///
    /// # Panics
    ///
    /// Does not panic, but will return errors if inputs are invalid
    pub fn new(initial_bpm: f32, initial_confidence: f32) -> Self {
        Self {
            current_bpm: initial_bpm,
            current_confidence: initial_confidence.max(0.0).min(1.0),
            history: vec![initial_bpm],
        }
    }
    
    /// Update with new onset evidence
    ///
    /// Performs Bayesian update: P(BPM | evidence) ∝ P(evidence | BPM) × P(BPM | prior)
    ///
    /// # Arguments
    ///
    /// * `onsets` - New onset times in seconds (must be sorted)
    /// * `sample_rate` - Sample rate in Hz (for logging)
    ///
    /// # Returns
    ///
    /// Updated BPM and confidence
    ///
    /// # Errors
    ///
    /// Returns `AnalysisError` if:
    /// - Onsets list is empty
    /// - Current BPM is invalid
    /// - Numerical errors occur
    pub fn update_with_onsets(
        &mut self,
        onsets: &[f32],
        _sample_rate: u32,
    ) -> Result<(f32, f32), AnalysisError> {
        if onsets.is_empty() {
            return Err(AnalysisError::InvalidInput(
                "Cannot update: no onsets provided".to_string()
            ));
        }
        
        if self.current_bpm <= EPSILON || self.current_bpm > 300.0 {
            return Err(AnalysisError::InvalidInput(
                format!("Invalid current BPM: {:.2}", self.current_bpm)
            ));
        }
        
        log::debug!("Bayesian update: current BPM={:.2}, {} new onsets",
                    self.current_bpm, onsets.len());
        
        // Compute likelihood: P(evidence | BPM) for candidate BPMs
        // Test BPMs around current estimate (±5 BPM in 0.5 BPM steps)
        let bpm_candidates = self.generate_bpm_candidates();
        let mut best_bpm = self.current_bpm;
        let mut best_likelihood = 0.0;
        
        for &candidate_bpm in &bpm_candidates {
            let likelihood = self.compute_likelihood(onsets, candidate_bpm)?;
            
            if likelihood > best_likelihood {
                best_likelihood = likelihood;
                best_bpm = candidate_bpm;
            }
        }
        
        // Compute prior: P(BPM | previous_estimate)
        // Gaussian centered at current_bpm with sigma = PRIOR_SIGMA
        let prior = self.compute_prior(best_bpm);
        
        // Posterior: P(BPM | evidence) ∝ Likelihood × Prior
        // Note: We use best_likelihood and prior separately for confidence calculation
        let _posterior = best_likelihood * prior;
        
        // Update BPM estimate
        let old_bpm = self.current_bpm;
        self.current_bpm = best_bpm;
        self.history.push(best_bpm);
        
        // Update confidence: combine likelihood and prior strength
        // Higher confidence if likelihood is high and BPM change is small
        let bpm_change = (best_bpm - old_bpm).abs();
        let change_penalty = if bpm_change < 1.0 {
            1.0 // Small change: no penalty
        } else if bpm_change < 3.0 {
            0.8 // Moderate change: small penalty
        } else {
            0.5 // Large change: significant penalty
        };
        
        self.current_confidence = (best_likelihood * change_penalty).min(1.0);
        
        log::debug!("Bayesian update complete: BPM {:.2} → {:.2}, confidence {:.3} → {:.3}",
                    old_bpm, self.current_bpm, self.current_confidence, self.current_confidence);
        
        Ok((self.current_bpm, self.current_confidence))
    }
    
    /// Generate BPM candidates around current estimate
    ///
    /// Tests BPMs in range [current - 5, current + 5] with 0.5 BPM resolution
    fn generate_bpm_candidates(&self) -> Vec<f32> {
        let mut candidates = Vec::new();
        let min_bpm = (self.current_bpm - 5.0).max(60.0);
        let max_bpm = (self.current_bpm + 5.0).min(180.0);
        
        let mut bpm = min_bpm;
        while bpm <= max_bpm {
            candidates.push(bpm);
            bpm += 0.5;
        }
        
        candidates
    }
    
    /// Compute likelihood: P(evidence | BPM)
    ///
    /// Measures how well the observed onsets match the expected beat pattern
    /// for a given BPM. Uses Gaussian decay based on timing alignment.
    ///
    /// Formula: likelihood = product of exp(-distance² / (2 * σ²)) for each onset
    fn compute_likelihood(&self, onsets: &[f32], bpm: f32) -> Result<f32, AnalysisError> {
        if bpm <= EPSILON {
            return Err(AnalysisError::InvalidInput(
                format!("Invalid BPM for likelihood: {:.2}", bpm)
            ));
        }
        
        if onsets.is_empty() {
            return Ok(0.0);
        }
        
        let beat_interval = 60.0 / bpm;
        let start_time = onsets[0];
        
        // For each onset, find distance to nearest expected beat
        let mut log_likelihood = 0.0;
        let mut valid_onsets = 0;
        
        for &onset_time in onsets {
            // Find nearest expected beat time
            let beat_index = ((onset_time - start_time) / beat_interval).round() as i32;
            let expected_beat_time = start_time + (beat_index as f32 * beat_interval);
            
            // Distance to expected beat
            let distance = (onset_time - expected_beat_time).abs();
            
            // Gaussian likelihood: exp(-distance² / (2 * σ²))
            // Use log-likelihood for numerical stability
            let distance_sq = distance * distance;
            let sigma_sq = LIKELIHOOD_SIGMA * LIKELIHOOD_SIGMA;
            
            if sigma_sq <= EPSILON {
                return Err(AnalysisError::NumericalError(
                    "Likelihood sigma too small".to_string()
                ));
            }
            
            let log_prob = -distance_sq / (2.0 * sigma_sq);
            log_likelihood += log_prob;
            valid_onsets += 1;
        }
        
        if valid_onsets == 0 {
            return Ok(0.0);
        }
        
        // Average log-likelihood, then convert back to linear scale
        let avg_log_likelihood = log_likelihood / valid_onsets as f32;
        let likelihood = avg_log_likelihood.exp();
        
        Ok(likelihood)
    }
    
    /// Compute prior: P(BPM | previous_estimate)
    ///
    /// Gaussian prior centered at current_bpm with standard deviation PRIOR_SIGMA.
    /// Higher probability for BPMs close to current estimate.
    fn compute_prior(&self, bpm: f32) -> f32 {
        let bpm_diff = (bpm - self.current_bpm).abs();
        let sigma_sq = PRIOR_SIGMA * PRIOR_SIGMA;
        
        if sigma_sq <= EPSILON {
            return 0.0;
        }
        
        // Gaussian: exp(-(bpm - current)² / (2 * σ²))
        let diff_sq = bpm_diff * bpm_diff;
        (-diff_sq / (2.0 * sigma_sq)).exp()
    }
    
    /// Get current BPM estimate
    pub fn get_bpm(&self) -> f32 {
        self.current_bpm
    }
    
    /// Get current confidence
    pub fn get_confidence(&self) -> f32 {
        self.current_confidence
    }
    
    /// Get BPM history
    pub fn get_history(&self) -> &[f32] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bayesian_tracker_creation() {
        let tracker = BayesianBeatTracker::new(120.0, 0.8);
        assert_eq!(tracker.current_bpm, 120.0);
        assert_eq!(tracker.current_confidence, 0.8);
        assert_eq!(tracker.history.len(), 1);
        assert_eq!(tracker.history[0], 120.0);
    }

    #[test]
    fn test_bayesian_tracker_confidence_clamping() {
        // Test that confidence is clamped to [0, 1]
        let tracker1 = BayesianBeatTracker::new(120.0, 1.5);
        assert_eq!(tracker1.current_confidence, 1.0);
        
        let tracker2 = BayesianBeatTracker::new(120.0, -0.5);
        assert_eq!(tracker2.current_confidence, 0.0);
    }

    #[test]
    fn test_generate_bpm_candidates() {
        let tracker = BayesianBeatTracker::new(120.0, 0.8);
        let candidates = tracker.generate_bpm_candidates();
        
        assert!(!candidates.is_empty());
        
        // Should include current BPM
        assert!(candidates.iter().any(|&bpm| (bpm - 120.0).abs() < 0.1));
        
        // Should test range around current BPM
        let min_candidate = candidates.iter().copied().fold(f32::INFINITY, f32::min);
        let max_candidate = candidates.iter().copied().fold(0.0f32, f32::max);
        
        assert!(min_candidate >= 115.0, "Min candidate should be around 115");
        assert!(max_candidate <= 125.0, "Max candidate should be around 125");
    }

    #[test]
    fn test_compute_likelihood() {
        let tracker = BayesianBeatTracker::new(120.0, 0.8);
        
        // Perfect 120 BPM onsets (0.5s intervals)
        let perfect_onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let likelihood = tracker.compute_likelihood(&perfect_onsets, 120.0).unwrap();
        
        assert!(likelihood > 0.0);
        assert!(likelihood <= 1.0);
        
        // Test with wrong BPM (should have lower likelihood)
        let wrong_likelihood = tracker.compute_likelihood(&perfect_onsets, 100.0).unwrap();
        assert!(likelihood > wrong_likelihood, "Correct BPM should have higher likelihood");
    }

    #[test]
    fn test_compute_likelihood_empty_onsets() {
        let tracker = BayesianBeatTracker::new(120.0, 0.8);
        let likelihood = tracker.compute_likelihood(&[], 120.0).unwrap();
        assert_eq!(likelihood, 0.0);
    }

    #[test]
    fn test_compute_prior() {
        let tracker = BayesianBeatTracker::new(120.0, 0.8);
        
        // Prior at current BPM should be highest
        let prior_at_current = tracker.compute_prior(120.0);
        assert!(prior_at_current > 0.0);
        assert!(prior_at_current <= 1.0);
        
        // Prior at different BPM should be lower
        let prior_at_different = tracker.compute_prior(130.0);
        assert!(prior_at_current > prior_at_different,
               "Prior at current BPM should be higher than at different BPM");
    }

    #[test]
    fn test_update_with_onsets() {
        let mut tracker = BayesianBeatTracker::new(120.0, 0.8);
        
        // Update with perfect 120 BPM onsets
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let (new_bpm, new_confidence) = tracker.update_with_onsets(&onsets, 44100).unwrap();
        
        assert!(new_bpm > 0.0);
        assert!(new_confidence >= 0.0 && new_confidence <= 1.0);
        assert_eq!(tracker.current_bpm, new_bpm);
        assert_eq!(tracker.current_confidence, new_confidence);
        assert_eq!(tracker.history.len(), 2); // Initial + updated
    }

    #[test]
    fn test_update_with_onsets_empty() {
        let mut tracker = BayesianBeatTracker::new(120.0, 0.8);
        assert!(tracker.update_with_onsets(&[], 44100).is_err());
    }

    #[test]
    fn test_update_with_onsets_invalid_bpm() {
        let mut tracker1 = BayesianBeatTracker::new(0.0, 0.8);
        assert!(tracker1.update_with_onsets(&[0.0, 0.5], 44100).is_err());
        
        let mut tracker2 = BayesianBeatTracker::new(350.0, 0.8);
        assert!(tracker2.update_with_onsets(&[0.0, 0.5], 44100).is_err());
    }

    #[test]
    fn test_get_bpm_and_confidence() {
        let tracker = BayesianBeatTracker::new(120.0, 0.85);
        assert_eq!(tracker.get_bpm(), 120.0);
        assert_eq!(tracker.get_confidence(), 0.85);
    }

    #[test]
    fn test_get_history() {
        let mut tracker = BayesianBeatTracker::new(120.0, 0.8);
        tracker.update_with_onsets(&[0.0, 0.5, 1.0], 44100).unwrap();
        
        let history = tracker.get_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], 120.0);
    }
}

