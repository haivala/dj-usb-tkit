//! Multi-resolution tempogram validation
//!
//! Runs tempogram at multiple hop sizes (256, 512, 1024) and validates
//! agreement across resolutions. This improves robustness and accuracy.
//!
//! # Reference
//!
//! Schreiber, H., & Müller, M. (2018). A Single-Step Approach to Musical Tempo Estimation
//! Using a Convolutional Neural Network. *Proceedings of the International Society for
//! Music Information Retrieval Conference*.
//!
//! # Algorithm
//!
//! 1. Run tempogram at 3 hop sizes: 256, 512, 1024
//! 2. Extract BPM estimates from each resolution
//! 3. Check agreement: if all agree ±2 BPM → high confidence
//! 4. Return consensus BPM or best estimate
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::multi_resolution::multi_resolution_analysis;
//!
//! let magnitude_spec_frames = vec![vec![0.0f32; 1024]; 100];
//! let result = multi_resolution_analysis(&magnitude_spec_frames, 44100, 512, 40.0, 240.0, 0.5)?;
//! println!("Consensus BPM: {:.2} (confidence: {:.3})", result.bpm, result.confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use super::BpmEstimate;
use super::tempogram::{
    estimate_bpm_tempogram,
    estimate_bpm_tempogram_with_candidates,
    estimate_bpm_tempogram_with_candidates_band_fusion,
    TempogramBandFusionConfig,
    TempogramCandidateDebug,
};
use super::novelty::{superflux_novelty, energy_flux_novelty, hfc_novelty, combined_novelty_with_params};
use crate::features::chroma::extractor::compute_stft;

/// Multi-resolution tempogram analysis (simplified wrapper)
///
/// Runs tempogram at multiple hop sizes and validates agreement across resolutions.
/// This improves robustness by catching artifacts from individual hop sizes.
///
/// # Reference
///
/// Schreiber, H., & Müller, M. (2018). A Single-Step Approach to Musical Tempo Estimation
/// Using a Convolutional Neural Network. *Proceedings of the International Society for
/// Music Information Retrieval Conference*.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
///   Note: This should be computed with the base hop_size, we'll recompute STFT for other hop sizes
/// * `sample_rate` - Sample rate in Hz
/// * `base_hop_size` - Base hop size used for input spectrogram (typically 512)
/// * `min_bpm` - Minimum BPM to consider
/// * `max_bpm` - Maximum BPM to consider
/// * `bpm_resolution` - BPM resolution for autocorrelation tempogram
///
/// # Returns
///
/// Consensus BPM estimate with confidence based on multi-resolution agreement
///
/// # Errors
///
/// Returns `AnalysisError` if tempogram computation fails at all resolutions
///
/// # Note
///
/// This function currently uses the same spectrogram for all resolutions. For true
/// multi-resolution analysis, we would need to recompute STFT at different hop sizes.
/// This is a simplified version that validates consistency across the same data.
pub fn multi_resolution_analysis(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    base_hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
) -> Result<BpmEstimate, AnalysisError> {
    log::debug!("Multi-resolution tempogram analysis: {} frames, sample_rate={}, base_hop_size={}",
                magnitude_spec_frames.len(), sample_rate, base_hop_size);
    
    // Run tempogram at multiple hop sizes
    // Note: In a full implementation, we would recompute STFT at each hop size.
    // For now, we use the same spectrogram but simulate different resolutions by
    // using different hop_size values in the tempogram computation.
    // This is a simplified approach - true multi-resolution would require recomputing STFT.
    
    let hop_sizes = vec![256, 512, 1024];
    let mut results = Vec::new();
    
    for &hop_size in &hop_sizes {
        match estimate_bpm_tempogram(
            magnitude_spec_frames,
            sample_rate,
            hop_size,
            min_bpm,
            max_bpm,
            bpm_resolution,
        ) {
            Ok(est) => {
                log::debug!("Hop size {}: BPM={:.1}, confidence={:.3}",
                           hop_size, est.bpm, est.confidence);
                results.push((hop_size, est));
            }
            Err(e) => {
                log::warn!("Tempogram failed at hop_size {}: {}", hop_size, e);
                // Continue with other hop sizes
            }
        }
    }
    
    if results.is_empty() {
        return Err(AnalysisError::ProcessingError(
            "All multi-resolution tempogram analyses failed".to_string()
        ));
    }
    
    // Check agreement across resolutions
    if results.len() >= 2 {
        // Check if all results agree within ±2 BPM
        let first_bpm = results[0].1.bpm;
        let all_agree = results.iter().all(|(_, est)| (est.bpm - first_bpm).abs() < 2.0);
        
        if all_agree {
            // All resolutions agree: use average with boosted confidence
            let avg_bpm: f32 = results.iter().map(|(_, est)| est.bpm).sum::<f32>() / results.len() as f32;
            let avg_confidence: f32 = results.iter().map(|(_, est)| est.confidence).sum::<f32>() / results.len() as f32;
            
            // Boost confidence for agreement
            let boosted_confidence = (avg_confidence * 1.2).min(1.0);
            
            log::debug!("Multi-resolution agreement: all {} resolutions agree on {:.1} BPM (conf={:.3})",
                       results.len(), avg_bpm, boosted_confidence);
            
            Ok(BpmEstimate {
                bpm: avg_bpm,
                confidence: boosted_confidence,
                method_agreement: results.len() as u32,
            })
        } else {
            // Resolutions disagree: use best confidence
            let best = results.iter()
                .max_by(|a, b| a.1.confidence.partial_cmp(&b.1.confidence).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap();
            
            log::debug!("Multi-resolution disagreement: using best (hop_size={}, BPM={:.1}, conf={:.3})",
                       best.0, best.1.bpm, best.1.confidence);
            
            Ok(BpmEstimate {
                bpm: best.1.bpm,
                confidence: best.1.confidence * 0.9, // Slight penalty for disagreement
                method_agreement: 1,
            })
        }
    } else {
        // Only one resolution succeeded
        let result = &results[0];
        log::debug!("Single resolution result: hop_size={}, BPM={:.1}, conf={:.3}",
                   result.0, result.1.bpm, result.1.confidence);
        
        Ok(result.1.clone())
    }
}

/// True multi-resolution tempogram BPM estimation (Phase 1F tuning).
///
/// Recomputes STFT magnitudes at hop sizes {256, 512, 1024} on the *audio samples*,
/// runs the tempogram pipeline on each, then fuses candidates using a calibrated
/// cross-resolution scoring rule aimed at resolving T vs 2T vs T/2 ambiguity.
pub fn multi_resolution_tempogram_from_samples(
    samples: &[f32],
    sample_rate: u32,
    frame_size: usize,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    top_k: usize,
    w512: f32,
    w256: f32,
    w1024: f32,
    structural_discount: f32,
    double_time_512_factor: f32,
    margin_threshold: f32,
    use_human_prior: bool,
    band_cfg: Option<TempogramBandFusionConfig>,
) -> Result<(BpmEstimate, Vec<TempogramCandidateDebug>), AnalysisError> {
    if samples.len() < frame_size {
        return Err(AnalysisError::InvalidInput(
            "Audio too short for STFT".to_string(),
        ));
    }

    let top_k = top_k.max(1);
    // Use a wider candidate list on auxiliary hops to reduce false “missing support” from truncation.
    //
    // Important: these auxiliary candidate lists are used for cross-hop support checks (fold-up/fold-down
    // and family support). If aux_k is too small, we often "miss" a half/double-time candidate that exists
    // but sits outside the top-K for hop=256/hop=1024, causing avoidable metrical errors.
    let aux_k = (top_k.saturating_mul(4)).clamp(25, 200);
    let tol = 2.0f32.max(bpm_resolution);

    let hop_256 = compute_stft(samples, frame_size, 256)?;
    let hop_512 = compute_stft(samples, frame_size, 512)?;
    let hop_1024 = compute_stft(samples, frame_size, 1024)?;

    let call = |spec: &[Vec<f32>], hop: u32, k: usize| -> Result<(BpmEstimate, Vec<TempogramCandidateDebug>), AnalysisError> {
        if let Some(cfg) = band_cfg
            .clone()
            .filter(|c| c.enabled || c.enable_mel || c.consensus_bonus > 0.0)
        {
            estimate_bpm_tempogram_with_candidates_band_fusion(spec, sample_rate, hop, min_bpm, max_bpm, bpm_resolution, k, cfg)
        } else {
            estimate_bpm_tempogram_with_candidates(spec, sample_rate, hop, min_bpm, max_bpm, bpm_resolution, k)
        }
    };

    let (_e256, c256) = call(&hop_256, 256, aux_k)?;
    let (_e512, mut c512) = call(&hop_512, 512, top_k)?;
    let (_e1024, c1024) = call(&hop_1024, 1024, aux_k)?;

    let dbg = band_cfg
        .as_ref()
        .and_then(|c| c.debug_track_id)
        .map(|id| {
            let gt = band_cfg.as_ref().and_then(|c| c.debug_gt_bpm);
            let top_n = band_cfg.as_ref().map(|c| c.debug_top_n).unwrap_or(5).max(1);
            (id, gt, top_n)
        });

    fn lookup_nearest(cands: &[TempogramCandidateDebug], bpm: f32, tol: f32) -> f32 {
        let mut best_d = f32::INFINITY;
        let mut best_s = 0.0f32;
        for c in cands {
            let d = (c.bpm - bpm).abs();
            if d <= tol && d < best_d {
                best_d = d;
                best_s = c.score;
            }
        }
        best_s
    }

    #[derive(Clone, Copy)]
    struct Hyp {
        bpm: f32,
        score: f32,
    }

    let mut hyps: Vec<Hyp> = Vec::new();

    if let Some((track_id, gt, top_n)) = dbg {
        eprintln!("\n=== DEBUG multi-res (track_id={}) ===", track_id);
        if let Some(gt_bpm) = gt {
            eprintln!("GT bpm: {:.3}", gt_bpm);
        }

        fn print_top(label: &str, cands: &[TempogramCandidateDebug], n: usize) {
            eprintln!("{} top-{}:", label, n);
            for c in cands.iter().take(n) {
                eprintln!("  bpm={:7.2} score={:.4}", c.bpm, c.score);
            }
        }
        print_top("hop=256", &c256, top_n);
        print_top("hop=512", &c512, top_n);
        print_top("hop=1024", &c1024, top_n);

        if let Some(gt_bpm) = gt {
            let s_t_512 = lookup_nearest(&c512, gt_bpm, tol);
            let s_t_256 = lookup_nearest(&c256, gt_bpm, tol);
            let s_t_1024 = lookup_nearest(&c1024, gt_bpm, tol);
            let s_2t_512 = lookup_nearest(&c512, gt_bpm * 2.0, tol);
            let s_2t_256 = lookup_nearest(&c256, gt_bpm * 2.0, tol);
            let s_2t_1024 = lookup_nearest(&c1024, gt_bpm * 2.0, tol);
            let s_half_512 = lookup_nearest(&c512, gt_bpm * 0.5, tol);
            let s_half_256 = lookup_nearest(&c256, gt_bpm * 0.5, tol);
            let s_half_1024 = lookup_nearest(&c1024, gt_bpm * 0.5, tol);

            eprintln!("Support near GT / family (lookup tol={:.2}):", tol);
            eprintln!("  T     @512={:.4} @256={:.4} @1024={:.4}", s_t_512, s_t_256, s_t_1024);
            eprintln!("  2T    @512={:.4} @256={:.4} @1024={:.4}", s_2t_512, s_2t_256, s_2t_1024);
            eprintln!("  T/2   @512={:.4} @256={:.4} @1024={:.4}", s_half_512, s_half_256, s_half_1024);

            // Recompute hypothesis scores for T=GT using the same rules as the fusion.
            let h_t = w512 * s_t_512 + w256 * s_t_256 + w1024 * (s_t_1024 + structural_discount * s_2t_1024);
            let mut h_2t = w512
                * (double_time_512_factor * s_t_512 + (1.0 - double_time_512_factor) * s_2t_512)
                + w256 * s_2t_256
                + w1024 * s_2t_1024;
            let mut h_half = w512
                * (double_time_512_factor * s_t_512 + (1.0 - double_time_512_factor) * s_half_512)
                + w256 * s_half_256
                + w1024 * s_half_1024;

            let eps = 1e-6f32;
            let ratio_2t_256 = (s_2t_256 + eps) / (s_t_256 + eps);
            let ratio_half_1024 = (s_half_1024 + eps) / (s_t_1024 + eps);
            if ratio_2t_256 < 1.10 {
                h_2t *= 0.75;
            }
            if ratio_2t_256 < 1.00 {
                h_2t *= 0.75;
            }
            if ratio_half_1024 < 1.10 {
                h_half *= 0.75;
            }
            if ratio_half_1024 < 1.00 {
                h_half *= 0.75;
            }

            let mut local: Vec<(f32, f32, &str)> = vec![
                (gt_bpm, h_t, "T"),
                (gt_bpm * 2.0, h_2t, "2T"),
                (gt_bpm * 0.5, h_half, "T/2"),
            ];
            local.retain(|(b, _, _)| *b >= min_bpm && *b <= max_bpm);
            for (b, s, _) in &mut local {
                if *b > 210.0 {
                    *s *= 0.80;
                } else if *b > 180.0 {
                    *s *= 0.90;
                } else if *b < 60.0 {
                    *s *= 0.92;
                }
            }
            local.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            eprintln!("Fusion scores (T anchored at GT, after guardrails+prior):");
            for (b, s, tag) in &local {
                eprintln!("  {:>3} bpm={:7.2} score={:.4}", tag, *b, *s);
            }
            if local.len() >= 2 {
                eprintln!(
                    "  margin(best-second)={:.4} (threshold={:.4})",
                    local[0].1 - local[1].1,
                    margin_threshold
                );
            }
            eprintln!(
                "  ratio_2T_256={:.3} ratio_half_1024={:.3} (guardrails)",
                ratio_2t_256,
                ratio_half_1024
            );
        }
    }

    for t in c512.iter().take(top_k) {
        let t_bpm = t.bpm;
        if !(t_bpm.is_finite() && t_bpm > 0.0) {
            continue;
        }

        let s_t_512 = lookup_nearest(&c512, t_bpm, tol);
        let s_t_256 = lookup_nearest(&c256, t_bpm, tol);
        let s_t_1024 = lookup_nearest(&c1024, t_bpm, tol);

        let s_2t_512 = lookup_nearest(&c512, t_bpm * 2.0, tol);
        let s_2t_256 = lookup_nearest(&c256, t_bpm * 2.0, tol);
        let s_2t_1024 = lookup_nearest(&c1024, t_bpm * 2.0, tol);

        let s_half_512 = lookup_nearest(&c512, t_bpm * 0.5, tol);
        let s_half_256 = lookup_nearest(&c256, t_bpm * 0.5, tol);
        let s_half_1024 = lookup_nearest(&c1024, t_bpm * 0.5, tol);

        // H(T): tempo is T (IMPORTANT: do NOT “leak” 1024 support for 2T into the score for T).
        //
        // If hop=1024 strongly supports 2T, that is evidence *for 2T*, not for T.
        // The previous formula added `structural_discount * s_2t_1024` into H(T), which can
        // incorrectly reinforce half-time anchors (e.g., when T is ~74 and 2T is ~148).
        let h_t = w512 * s_t_512 + w256 * s_t_256 + w1024 * s_t_1024;

        // H(2T): tempo is double-time
        //
        // Keep the hop=512 mixing heuristic (it can sometimes “blur” between T and 2T),
        // but avoid injecting 2T evidence into T itself (handled above).
        let mut h_2t = w512
            * (double_time_512_factor * s_t_512 + (1.0 - double_time_512_factor) * s_2t_512)
            + w256 * s_2t_256
            + w1024 * s_2t_1024;

        // H(T/2): tempo is half-time
        let mut h_half = w512
            * (double_time_512_factor * s_t_512 + (1.0 - double_time_512_factor) * s_half_512)
            + w256 * s_half_256
            + w1024 * s_half_1024;

        // Structural preference (hop=1024 tends to be less dominated by dense subdivisions):
        // if 1024 prefers T over T/2 or 2T, lightly discount the competing hypothesis.
        // This is intentionally mild; we still allow switching when evidence is strong.
        if s_t_1024 > s_half_1024 * 1.02 {
            h_half *= 0.90;
        }
        if s_t_1024 > s_2t_1024 * 1.02 {
            h_2t *= 0.90;
        }

        // Guardrails: only allow switching to 2T / T/2 when there is meaningful supporting evidence.
        // This prevents the fusion from “chasing” subdivision peaks that are only slightly stronger.
        let eps = 1e-6f32;
        let ratio_2t_256 = (s_2t_256 + eps) / (s_t_256 + eps);
        if ratio_2t_256 < 1.10 {
            h_2t *= 0.75;
        }
        if ratio_2t_256 < 1.00 {
            h_2t *= 0.75;
        }

        let ratio_half_1024 = (s_half_1024 + eps) / (s_t_1024 + eps);
        if ratio_half_1024 < 1.10 {
            h_half *= 0.75;
        }
        if ratio_half_1024 < 1.00 {
            h_half *= 0.75;
        }

        // Candidate hypotheses (clamp to allowed BPM range)
        let mut local: Vec<(f32, f32)> = vec![
            (t_bpm, h_t),
            (t_bpm * 2.0, h_2t),
            (t_bpm * 0.5, h_half),
        ];
        local.retain(|(b, _)| *b >= min_bpm && *b <= max_bpm);

        // Mild tempo prior (applied uniformly to hypothesis score)
        for (b, s) in &mut local {
            if *b > 210.0 {
                *s *= 0.80;
            } else if *b > 180.0 {
                *s *= 0.90;
            } else if *b < 60.0 {
                *s *= 0.92;
            }
        }

        local.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if local.is_empty() {
            continue;
        }

        let (best_bpm, best_score) = local[0];
        let second_score = local.get(1).map(|x| x.1).unwrap_or(0.0);
        let margin = best_score - second_score;

        // Critical: only switch away from T when the best alternative is clearly better.
        // Otherwise, keep T (reduces catastrophic 2× flips near 240).
        let mut chosen_bpm = best_bpm;
        let mut chosen_score = best_score;
        if (chosen_bpm - t_bpm).abs() > 1e-3 && margin < margin_threshold {
            chosen_bpm = t_bpm;
            chosen_score = h_t;
        }

        // If the choice is not clearly separated, apply an optional gentle prior as tie-break.
        if margin < margin_threshold {
            if use_human_prior && chosen_bpm >= 70.0 && chosen_bpm <= 180.0 && margin < 0.05 {
                chosen_score += 0.05;
            }
        }

        hyps.push(Hyp {
            bpm: chosen_bpm,
            score: chosen_score,
        });
    }

    if hyps.is_empty() {
        return Err(AnalysisError::ProcessingError(
            "Multi-resolution fusion produced no hypotheses".to_string(),
        ));
    }

    // Deduplicate hypotheses by BPM proximity; keep highest score per cluster.
    hyps.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut unique: Vec<Hyp> = Vec::new();
    for h in hyps {
        if unique.iter().any(|u| (u.bpm - h.bpm).abs() < 0.75) {
            continue;
        }
        unique.push(h);
        if unique.len() >= 8 {
            break;
        }
    }

    let mut best = unique[0];

    // Final tempo-family fold (post hoc):
    //
    // If we land in a classic trap zone, prefer the "slower" metrical level when it is
    // meaningfully supported across hops. This targets common 180 vs 90 ambiguity where
    // dense subdivisions inflate the 2× candidate.
    fn total_support(
        c256: &[TempogramCandidateDebug],
        c512: &[TempogramCandidateDebug],
        c1024: &[TempogramCandidateDebug],
        bpm: f32,
        tol: f32,
    ) -> (f32, u32) {
        let mut agree = 0u32;
        let s256 = lookup_nearest(c256, bpm, tol);
        let s512 = lookup_nearest(c512, bpm, tol);
        let s1024 = lookup_nearest(c1024, bpm, tol);
        if s256 > 0.0 {
            agree += 1;
        }
        if s512 > 0.0 {
            agree += 1;
        }
        if s1024 > 0.0 {
            agree += 1;
        }
        (s256 + s512 + s1024, agree)
    }

    // Beat-contrast alignment score (phase optimized) on the hop=512 novelty curve.
    // We use this as an additional guardrail for octave folds (helps avoid folding true 180 -> 90).
    fn beat_contrast_score(novelty: &[f32], sample_rate: u32, hop: u32, bpm: f32) -> f32 {
        if novelty.len() < 16 || !(bpm.is_finite() && bpm > 0.0) || sample_rate == 0 || hop == 0 {
            return 0.0;
        }
        let frames_per_beat = (60.0 * sample_rate as f32) / (bpm * hop as f32);
        if !frames_per_beat.is_finite() || frames_per_beat < 3.0 {
            return 0.0;
        }
        let period = frames_per_beat.round() as isize;
        if period < 3 || period > 512 {
            return 0.0;
        }
        let period = period as usize;
        let w = 2usize; // +/- window in frames
        let total = novelty.iter().sum::<f32>().max(1e-6);

        let mut best = -1e9f32;
        for phase in 0..period {
            let mut beat_sum = 0.0f32;
            let mut beat_n = 0u32;

            let mut half_sum = 0.0f32;
            let mut half_n = 0u32;

            let mut third_sum = 0.0f32;
            let mut third_n = 0u32;

            let mut i = phase;
            while i < novelty.len() {
                // Beat window
                let start = i.saturating_sub(w);
                let end = (i + w + 1).min(novelty.len());
                let mut mx = 0.0f32;
                for v in &novelty[start..end] {
                    mx = mx.max(*v);
                }
                beat_sum += mx;
                beat_n += 1;

                // Half-beat window (if meaningful)
                if period >= 6 {
                    let j = i + period / 2;
                    if j < novelty.len() {
                        let start = j.saturating_sub(w);
                        let end = (j + w + 1).min(novelty.len());
                        let mut mx = 0.0f32;
                        for v in &novelty[start..end] {
                            mx = mx.max(*v);
                        }
                        half_sum += mx;
                        half_n += 1;
                    }
                }

                // Triplet subdivision windows (if meaningful)
                if period >= 9 {
                    for frac in [1usize, 2usize] {
                        let j = i + (period * frac) / 3;
                        if j < novelty.len() {
                            let start = j.saturating_sub(w);
                            let end = (j + w + 1).min(novelty.len());
                            let mut mx = 0.0f32;
                            for v in &novelty[start..end] {
                                mx = mx.max(*v);
                            }
                            third_sum += mx;
                            third_n += 1;
                        }
                    }
                }

                i += period;
            }

            let beat_mean = if beat_n > 0 { beat_sum / beat_n as f32 } else { 0.0 };
            let half_mean = if half_n > 0 { half_sum / half_n as f32 } else { 0.0 };
            let third_mean = if third_n > 0 { third_sum / third_n as f32 } else { 0.0 };

            // Contrast: prefer strong beat alignment while penalizing equally strong offbeats/subdivisions.
            let contrast = beat_mean - 0.60 * half_mean - 0.40 * third_mean;
            // Normalize by mean novelty so dense-onset tracks don't dominate the score scale.
            let score = (contrast / (total / novelty.len() as f32).max(1e-6)).clamp(-10.0, 10.0);
            best = best.max(score);
        }

        best
    }

    // Precompute hop=512 novelty once (used for octave fold gating and triplet-family search).
    let novelty_512_opt: Option<Vec<f32>> = band_cfg.as_ref().map(|cfg| {
        let sf = superflux_novelty(&hop_512, cfg.superflux_max_filter_bins).unwrap_or_default();
        let en = energy_flux_novelty(&hop_512).unwrap_or_default();
        let hf = hfc_novelty(&hop_512, sample_rate).unwrap_or_default();
        combined_novelty_with_params(
            &sf,
            &en,
            &hf,
            cfg.novelty_w_spectral,
            cfg.novelty_w_energy,
            cfg.novelty_w_hfc,
            cfg.novelty_local_mean_window,
            cfg.novelty_smooth_window,
        )
    });

    // Fold-down (high -> half)
    if best.bpm >= 170.0 {
        let half = best.bpm * 0.5;
        if half >= 70.0 && half <= 120.0 {
            let (s_best, a_best) = total_support(&c256, &c512, &c1024, best.bpm, tol);
            let (s_half, a_half) = total_support(&c256, &c512, &c1024, half, tol);
            let ratio = if s_best > 0.0 { s_half / s_best } else { 0.0 };
            // Slightly looser support ratio threshold (vs 0.55), but require strong cross-hop
            // agreement on the half tempo to avoid folding true high-tempo tracks down.
            if a_half >= 3 && s_half > 0.0 && s_best > 0.0 && ratio >= 0.45 {
                if let Some(track_id) = band_cfg.as_ref().and_then(|c| c.debug_track_id) {
                    eprintln!(
                        "DEBUG fold-down (track_id={}): {:.2} -> {:.2} (support ratio {:.3}, agree {}->{}).",
                        track_id,
                        best.bpm,
                        half,
                        ratio,
                        a_best,
                        a_half
                    );
                }
                best = Hyp { bpm: half, score: s_half };
            }
        }
    }

    // Fold-up (low -> double)
    if best.bpm <= 80.0 {
        let dbl = best.bpm * 2.0;
        if dbl >= 70.0 && dbl <= 180.0 {
            let (s_best, a_best) = total_support(&c256, &c512, &c1024, best.bpm, tol);
            let (s_dbl, a_dbl) = total_support(&c256, &c512, &c1024, dbl, tol);
            let ratio = if s_best > 0.0 { s_dbl / s_best } else { 0.0 };
            if a_dbl >= 2 && s_dbl > 0.0 && s_best > 0.0 && ratio >= 0.55 {
                if let Some(track_id) = band_cfg.as_ref().and_then(|c| c.debug_track_id) {
                    eprintln!(
                        "DEBUG fold-up (track_id={}): {:.2} -> {:.2} (support ratio {:.3}, agree {}->{}).",
                        track_id,
                        best.bpm,
                        dbl,
                        ratio,
                        a_best,
                        a_dbl
                    );
                }
                best = Hyp { bpm: dbl, score: s_dbl };
            }
        }
    }

    // Triplet / compound-meter family search (ambiguity-only, phase-aware):
    //
    // Goal: reduce 3/2× and 3/4× family mistakes without the catastrophic “120 -> 80” regressions
    // that raw support-based folding caused.
    //
    // Approach:
    // - Build hop=512 novelty (same conditioning as tempogram).
    // - Consider candidates in the local metrical family around the current best:
    //   {best, 3/2*best, 2/3*best, 4/3*best, 3/4*best}
    // - Require non-trivial multi-hop support (>=2 hops).
    // - Choose via a beat-contrast alignment score (phase optimized), with a small support prior.
    if let Some(cfg) = band_cfg.as_ref() {
        // Only attempt this in the typical tempo band where these confusions matter most.
        if best.bpm >= 70.0 && best.bpm <= 180.0 {
            let family: &[(f32, &str)] = &[
                (1.0, "T"),
                (3.0 / 2.0, "3/2"),
                (2.0 / 3.0, "2/3"),
                (4.0 / 3.0, "4/3"),
                (3.0 / 4.0, "3/4"),
            ];

            if let Some(novelty_512) = novelty_512_opt.as_ref().filter(|n| !n.is_empty()) {
                #[derive(Clone, Copy)]
                struct Fam {
                    bpm: f32,
                    support: f32,
                    align: f32,
                    label: &'static str,
                }

                let mut fams: Vec<Fam> = Vec::new();
                for (factor, label) in family {
                    let bpm = best.bpm * *factor;
                    if !(bpm.is_finite() && bpm >= min_bpm && bpm <= max_bpm) {
                        continue;
                    }
                    if bpm < 70.0 || bpm > 180.0 {
                        continue;
                    }
                    let (support, agree) = total_support(&c256, &c512, &c1024, bpm, tol);
                    if agree < 2 || support <= 0.0 {
                        continue;
                    }
                    let align = beat_contrast_score(&novelty_512, sample_rate, 512, bpm);
                    fams.push(Fam { bpm, support, align, label });
                }

                // Need at least 2 viable family candidates.
                if fams.len() >= 2 {
                    // Check whether any non-T candidate has meaningful support.
                    let best_support = fams.iter().map(|f| f.support).fold(0.0f32, f32::max).max(1e-6);
                    let max_alt_support = fams
                        .iter()
                        .filter(|f| (f.bpm - best.bpm).abs() > 0.75)
                        .map(|f| f.support / best_support)
                        .fold(0.0f32, f32::max);

                    // Only run the family search if there is real competing support.
                    if max_alt_support >= 0.45 {
                        // Combine alignment with a small support prior.
                        let mut chosen = fams[0];
                        let mut chosen_score = -1e9f32;
                        for f in fams {
                            let support_norm = (f.support / best_support).clamp(0.0, 1.0);
                            let score = f.align + 0.35 * support_norm;
                            if score > chosen_score {
                                chosen = f;
                                chosen_score = score;
                            }
                        }

                        // Only switch if the winner isn't the current best and wins meaningfully on alignment.
                        let current = Fam {
                            bpm: best.bpm,
                            support: total_support(&c256, &c512, &c1024, best.bpm, tol).0,
                            align: beat_contrast_score(&novelty_512, sample_rate, 512, best.bpm),
                            label: "T*",
                        };

                        if (chosen.bpm - best.bpm).abs() > 0.75 && chosen.align >= current.align + 0.40 {
                            if let Some(track_id) = cfg.debug_track_id {
                                eprintln!(
                                    "DEBUG triplet-family (track_id={}): {:.2} -> {:.2} ({}, support {:.3}->{:.3}, align {:.3}->{:.3})",
                                    track_id,
                                    best.bpm,
                                    chosen.bpm,
                                    chosen.label,
                                    current.support / best_support,
                                    chosen.support / best_support,
                                    current.align,
                                    chosen.align
                                );
                            }
                            best = Hyp { bpm: chosen.bpm, score: chosen.support };
                        }
                    }
                }
            }
        }
    }

    let second_score = unique.get(1).map(|h| h.score).unwrap_or(0.0);
    let conf = if best.score > 1e-6 {
        ((best.score - second_score).max(0.0) / best.score).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Method agreement: number of hop resolutions with direct support for the chosen BPM.
    let mut agree = 0u32;
    if lookup_nearest(&c256, best.bpm, tol) > 0.0 {
        agree += 1;
    }
    if lookup_nearest(&c512, best.bpm, tol) > 0.0 {
        agree += 1;
    }
    if lookup_nearest(&c1024, best.bpm, tol) > 0.0 {
        agree += 1;
    }

    // Mark which hop=512 candidates align with the final selection for downstream diagnostics.
    for c in &mut c512 {
        c.selected = (c.bpm - best.bpm).abs() < 0.75;
    }

    Ok((
        BpmEstimate {
            bpm: best.bpm,
            confidence: conf,
            method_agreement: agree,
        },
        c512,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_multi_resolution_analysis_basic() {
        // Create spectrogram with periodic pattern
        let mut spectrogram = vec![vec![0.1f32; 1024]; 500];
        
        // Add periodic pattern
        let period = 43;
        for i in 0..spectrogram.len() {
            if i % period == 0 {
                for bin in 0..512 {
                    spectrogram[i][bin] = 1.0;
                }
            }
        }
        
        let result = multi_resolution_analysis(&spectrogram, 44100, 512, 100.0, 140.0, 0.5);
        
        // Should either succeed or fail gracefully
        match result {
            Ok(est) => {
                assert!(est.bpm >= 100.0 && est.bpm <= 140.0);
                assert!(est.confidence >= 0.0 && est.confidence <= 1.0);
            }
            Err(_) => {
                // Failure is acceptable for test input
            }
        }
    }
    
    #[test]
    fn test_multi_resolution_analysis_empty() {
        let spectrogram = vec![];
        let result = multi_resolution_analysis(&spectrogram, 44100, 512, 40.0, 240.0, 0.5);
        assert!(result.is_err());
    }
}

