//! Period estimation modules
//!
//! BPM detection using two approaches:
//! - **Tempogram (NEW)**: Global temporal analysis using Fourier tempogram (85%+ accuracy)
//!   - Novelty curve extraction (spectral flux, energy flux, HFC)
//!   - FFT tempogram (frequency-domain analysis)
//!   - Autocorrelation tempogram (direct hypothesis testing)
//!   - Multi-resolution validation
//! - **Legacy methods (deprecated)**: Frame-by-frame analysis (30% accuracy)
//!   - Autocorrelation
//!   - Comb filterbank
//!   - Candidate filtering and merging
//!
//! # Example (Tempogram - Recommended)
//!
//! ```no_run
//! use stratum_dsp::features::period::tempogram::estimate_bpm_tempogram;
//!
//! let magnitude_spec_frames = vec![vec![0.0f32; 1024]; 100];
//! let result = estimate_bpm_tempogram(&magnitude_spec_frames, 44100, 512, 40.0, 240.0, 0.5)?;
//! println!("BPM: {:.2} (confidence: {:.3})", result.bpm, result.confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```
//!
//! # Example (Legacy - Deprecated)
//!
//! ```no_run
//! use stratum_dsp::features::period::estimate_bpm;
//!
//! let onsets = vec![0, 11025, 22050, 33075]; // 120 BPM at 44.1kHz
//! if let Some(estimate) = estimate_bpm(&onsets, 44100, 512, 60.0, 180.0, 1.0)? {
//!     println!("BPM: {:.2} (confidence: {:.3})", estimate.bpm, estimate.confidence);
//! }
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

// Legacy modules (deprecated, will be removed in v0.9.2)
pub mod autocorrelation;
pub mod candidate_filter;
pub mod comb_filter;
pub mod peak_picking;

// New tempogram modules (Phase 1F)
pub mod novelty;
pub mod tempogram_autocorr;
pub mod tempogram_fft;
pub mod tempogram;
pub mod multi_resolution;

// Legacy exports (deprecated)
pub use autocorrelation::estimate_bpm_from_autocorrelation;
pub use comb_filter::{estimate_bpm_from_comb_filter, coarse_to_fine_search};
pub use candidate_filter::merge_bpm_candidates;
pub use peak_picking::find_peaks;

// New tempogram exports (Phase 1F)
pub use tempogram::estimate_bpm_tempogram;
pub use multi_resolution::multi_resolution_analysis;

/// BPM candidate with confidence
#[derive(Debug, Clone)]
pub struct BpmCandidate {
    /// BPM estimate
    pub bpm: f32,
    
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// Final BPM estimate with method agreement
#[derive(Debug, Clone)]
pub struct BpmEstimate {
    /// BPM estimate
    pub bpm: f32,
    
    /// Confidence score
    pub confidence: f32,
    
    /// Number of methods that agree
    pub method_agreement: u32,
}

/// Soft guardrails for legacy BPM confidence.
///
/// These guardrails do **not** hard-clamp BPM values; they **cap** confidence for
/// legacy estimates that fall outside a preferred tempo range.
#[derive(Debug, Clone, Copy)]
pub struct LegacyBpmGuardrails {
    /// Preferred tempo lower bound (full confidence cap applies within preferred band).
    pub preferred_min: f32,
    /// Preferred tempo upper bound (full confidence cap applies within preferred band).
    pub preferred_max: f32,
    /// Soft tempo lower bound (medium confidence cap applies within soft-but-not-preferred band).
    pub soft_min: f32,
    /// Soft tempo upper bound (medium confidence cap applies within soft-but-not-preferred band).
    pub soft_max: f32,
    /// Confidence multiplier applied within the preferred band.
    pub mul_preferred: f32,
    /// Confidence multiplier applied within the soft band (outside preferred).
    pub mul_soft: f32,
    /// Confidence multiplier applied outside the soft band.
    pub mul_extreme: f32,
}

impl LegacyBpmGuardrails {
    fn mul_for(&self, bpm: f32) -> f32 {
        if !bpm.is_finite() {
            return 0.0;
        }
        if bpm >= self.preferred_min && bpm <= self.preferred_max {
            self.mul_preferred
        } else if bpm >= self.soft_min && bpm <= self.soft_max {
            self.mul_soft
        } else {
            self.mul_extreme
        }
    }

    fn clamp_sane(&self) -> Self {
        // Ensure sane ordering and multipliers are finite/non-negative.
        let preferred_min = self.preferred_min.min(self.preferred_max);
        let preferred_max = self.preferred_min.max(self.preferred_max);
        // Ensure preferred range lies inside soft range as much as possible.
        let soft_min = self.soft_min.min(self.soft_max).min(preferred_min);
        let soft_max = self.soft_min.max(self.soft_max).max(preferred_max);

        Self {
            preferred_min,
            preferred_max,
            soft_min,
            soft_max,
            mul_preferred: if self.mul_preferred.is_finite() { self.mul_preferred.max(0.0) } else { 0.0 },
            mul_soft: if self.mul_soft.is_finite() { self.mul_soft.max(0.0) } else { 0.0 },
            mul_extreme: if self.mul_extreme.is_finite() { self.mul_extreme.max(0.0) } else { 0.0 },
        }
    }
}

/// Estimate BPM from onset list using both autocorrelation and comb filterbank
///
/// This is the main public API for period estimation. It combines results from
/// both autocorrelation and comb filterbank methods, handling octave errors
/// and boosting confidence when methods agree.
///
/// # Arguments
///
/// * `onsets` - Onset times in samples
/// * `sample_rate` - Sample rate in Hz
/// * `hop_size` - Hop size used for onset detection
/// * `min_bpm` - Minimum BPM to consider (default: 60.0)
/// * `max_bpm` - Maximum BPM to consider (default: 180.0)
/// * `bpm_resolution` - BPM resolution for comb filterbank (default: 1.0)
///
/// # Returns
///
/// Best BPM estimate with confidence and method agreement, or None if no valid estimate found
///
/// # Errors
///
/// Returns `AnalysisError` if estimation fails
pub fn estimate_bpm(
    onsets: &[usize],
    sample_rate: u32,
    hop_size: usize,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
) -> Result<Option<BpmEstimate>, crate::error::AnalysisError> {
    estimate_bpm_internal(
        onsets,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
        bpm_resolution,
        None,
    )
}

/// Estimate BPM from onset list using legacy methods, with optional confidence guardrails.
///
/// Guardrails apply *soft confidence caps* based on tempo range to discourage selecting
/// extreme tempos unless the evidence is overwhelming.
pub fn estimate_bpm_with_guardrails(
    onsets: &[usize],
    sample_rate: u32,
    hop_size: usize,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    guardrails: LegacyBpmGuardrails,
) -> Result<Option<BpmEstimate>, crate::error::AnalysisError> {
    estimate_bpm_internal(
        onsets,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
        bpm_resolution,
        Some(guardrails),
    )
}

fn estimate_bpm_internal(
    onsets: &[usize],
    sample_rate: u32,
    hop_size: usize,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    guardrails: Option<LegacyBpmGuardrails>,
) -> Result<Option<BpmEstimate>, crate::error::AnalysisError> {
    use candidate_filter::merge_bpm_candidates;
    
    // Get candidates from both methods
    let autocorr_candidates = autocorrelation::estimate_bpm_from_autocorrelation(
        onsets,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
    )?;

    let comb_candidates = comb_filter::estimate_bpm_from_comb_filter(
        onsets,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
        bpm_resolution,
    )?;

    // Debug: Show top candidates from each method
    log::debug!("=== BPM CANDIDATES BEFORE MERGING ===");
    log::debug!("Autocorrelation top 5:");
    for (i, cand) in autocorr_candidates.iter().take(5).enumerate() {
        log::debug!("  {}. {:.2} BPM (confidence: {:.3})", i + 1, cand.bpm, cand.confidence);
    }
    log::debug!("Comb filter top 5:");
    for (i, cand) in comb_candidates.iter().take(5).enumerate() {
        log::debug!("  {}. {:.2} BPM (confidence: {:.3})", i + 1, cand.bpm, cand.confidence);
    }

    let guardrails = guardrails.map(|g| g.clamp_sane());

    // Get autocorrelation's top "preferred" candidate before merging (for post-merge preference).
    // If guardrails are active, this uses the preferred range; otherwise falls back to 60–180.
    let (preferred_min, preferred_max) = guardrails
        .map(|g| (g.preferred_min, g.preferred_max))
        .unwrap_or((60.0, 180.0));
    let autocorr_top_preferred = autocorr_candidates
        .iter()
        .find(|c| c.bpm >= preferred_min && c.bpm <= preferred_max)
        .map(|c| c.bpm);
    log::debug!(
        "Autocorr top preferred BPM before merge (range {:.1}-{:.1}): {:?}",
        preferred_min,
        preferred_max,
        autocorr_top_preferred
    );
    
    // Merge candidates
    let merged = merge_bpm_candidates(autocorr_candidates, comb_candidates, 50.0)?;
    
    // Debug: Show merged results
    log::debug!("=== MERGED BPM ESTIMATES ===");
    for (i, est) in merged.iter().take(5).enumerate() {
        log::debug!("  {}. {:.2} BPM (confidence: {:.3}, agreement: {})", 
                   i + 1, est.bpm, est.confidence, est.method_agreement);
    }

    // Return best estimate (take first, then we'll log diagnostics)
    let mut merged_vec: Vec<_> = merged.into_iter().collect();

    // Apply legacy guardrail confidence multipliers (legacy only).
    if let Some(g) = guardrails {
        for est in merged_vec.iter_mut() {
            let mul = g.mul_for(est.bpm);
            let before = est.confidence;
            est.confidence *= mul;
            log::debug!(
                "Legacy guardrail multiplier applied: {:.2} BPM (confidence {:.3} * {:.3} -> {:.3})",
                est.bpm,
                before,
                mul,
                est.confidence
            );
        }
        // Re-sort after capping to maintain descending-confidence order.
        merged_vec.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    
    // Special case: If autocorrelation's top preferred result exists, prefer it
    // This fixes cases where autocorr finds the right answer but comb subharmonics win
    if let Some(autocorr_top_preferred_bpm) = autocorr_top_preferred {
        if let Some(matching_idx) = merged_vec
            .iter()
            .position(|e| (e.bpm - autocorr_top_preferred_bpm).abs() < 2.0)
        {
            // Move matching estimate to front
            let matching = merged_vec.remove(matching_idx);
            merged_vec.insert(0, matching);
            log::debug!(
                "Preferring autocorr top preferred result: {:.2} BPM",
                autocorr_top_preferred_bpm
            );
        }
    }
    
    let best = merged_vec.first().cloned();
    if let Some(ref est) = best {
        log::debug!("=== FINAL BPM SELECTION ===");
        log::debug!("Winner: {:.2} BPM (confidence: {:.3}, agreement: {})", 
                   est.bpm, est.confidence, est.method_agreement);
        
        // Show runners-up for context
        if merged_vec.len() > 1 {
            log::debug!("Runners-up:");
            for (i, cand) in merged_vec.iter().take(4).enumerate().skip(1) {
                log::debug!("  {}. {:.2} BPM (confidence: {:.3}, agreement: {})", 
                           i + 1, cand.bpm, cand.confidence, cand.method_agreement);
            }
        }
        
        // Explain why it won
        let mut reasons = Vec::new();
        if let Some(g) = guardrails {
            if est.bpm >= g.preferred_min && est.bpm <= g.preferred_max {
                reasons.push(format!(
                    "in preferred range ({:.0}-{:.0} BPM)",
                    g.preferred_min, g.preferred_max
                ));
            } else if est.bpm >= g.soft_min && est.bpm <= g.soft_max {
                reasons.push(format!(
                    "in soft range ({:.0}-{:.0} BPM)",
                    g.soft_min, g.soft_max
                ));
            } else {
                reasons.push(format!(
                    "OUTSIDE soft range ({:.0}-{:.0} BPM)",
                    g.soft_min, g.soft_max
                ));
            }
        } else if est.bpm >= 60.0 && est.bpm <= 180.0 {
            reasons.push("in reasonable range (60-180 BPM)".to_string());
        }
        if est.method_agreement >= 5 {
            reasons.push(format!("high method agreement ({})", est.method_agreement));
        }
        if est.confidence > 1.0 {
            reasons.push(format!("boosted confidence ({:.3})", est.confidence));
        }
        if guardrails.is_none() && (est.bpm < 60.0 || est.bpm > 180.0) {
            reasons.push("OUT OF RANGE (penalized)".to_string());
        }
        
        if !reasons.is_empty() {
            log::debug!("  → Won because: {}", reasons.join(", "));
        } else {
            log::debug!("  → Won: highest confidence");
        }
    }
    Ok(best)
}

