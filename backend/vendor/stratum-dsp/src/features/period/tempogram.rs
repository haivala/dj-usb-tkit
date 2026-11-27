//! Tempogram-based BPM detection (main entry point)
//!
//! Combines FFT and autocorrelation tempogram methods for maximum accuracy.
//! Compares both methods and selects the best estimate or uses ensemble approach.
//!
//! # Reference
//!
//! Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
//! *IEEE Transactions on Audio, Speech, and Language Processing*.
//!
//! # Algorithm
//!
//! 1. Extract novelty curve from magnitude spectrogram
//! 2. Compute FFT tempogram (frequency-domain analysis)
//! 3. Compute autocorrelation tempogram (direct hypothesis testing)
//! 4. Compare results and select best estimate or use ensemble
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::tempogram::estimate_bpm_tempogram;
//!
//! let magnitude_spec_frames = vec![vec![0.0f32; 1024]; 100];
//! let result = estimate_bpm_tempogram(&magnitude_spec_frames, 44100, 512, 40.0, 240.0, 0.5)?;
//! println!("BPM: {:.2} (confidence: {:.3})", result.bpm, result.confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use super::BpmEstimate;
use super::novelty::{
    superflux_novelty,
    superflux_novelty_band,
    mel_superflux_novelty,
    energy_flux_novelty,
    energy_flux_novelty_band,
    hfc_novelty,
    hfc_novelty_band,
    combined_novelty,
    combined_novelty_with_params,
};
use super::tempogram_autocorr::{autocorrelation_tempogram, find_best_bpm_autocorr};
use super::tempogram_fft::{fft_tempogram, find_best_bpm_fft};

/// Configuration for band-limited novelty fusion inside the tempogram BPM estimator.
///
/// This targets the dominant remaining bottleneck we’ve observed empirically:
/// **candidate generation** (GT not present in top-N candidates), especially in the
/// 60–90 and 120–150 BPM ambiguity bands.
#[derive(Debug, Clone)]
pub struct TempogramBandFusionConfig {
    /// Enable multi-band novelty + tempogram fusion.
    pub enabled: bool,
    /// Upper cutoff for "low" band in Hz (inclusive-ish; converted to bins).
    pub low_max_hz: f32,
    /// Upper cutoff for "mid" band in Hz (high band starts at this).
    pub mid_max_hz: f32,
    /// Upper cutoff for "high" band in Hz. If <= 0, uses Nyquist.
    pub high_max_hz: f32,
    /// Weight for the full-band tempogram contribution when band-score fusion is enabled.
    pub w_full: f32,
    /// Weight for the low band contribution.
    pub w_low: f32,
    /// Weight for the mid band contribution.
    pub w_mid: f32,
    /// Weight for the high band contribution.
    pub w_high: f32,
    /// If true, bands contribute only to candidate seeding; scoring uses full-band only.
    pub seed_only: bool,
    /// Normalized support threshold for counting a band as supporting a candidate (0..1).
    pub support_threshold: f32,
    /// Consensus bonus applied when multiple bands support the same candidate.
    pub consensus_bonus: f32,
    /// Enable log-mel novelty tempogram as an additional candidate generator/support signal.
    pub enable_mel: bool,
    /// Mel band count used by log-mel novelty.
    pub mel_n_mels: usize,
    /// Minimum mel frequency (Hz).
    pub mel_fmin_hz: f32,
    /// Maximum mel frequency (Hz). If <= 0, uses Nyquist.
    pub mel_fmax_hz: f32,
    /// Max-filter neighborhood radius in mel bins (SuperFlux-style reference).
    pub mel_max_filter_bins: usize,
    /// Weight for mel variant when `seed_only=false` (scoring fusion). In seed-only mode, this
    /// is still used for logging/diagnostics, but scoring remains full-band-only.
    pub w_mel: f32,
    /// Novelty weights for combining {spectral, energy, HFC}.
    pub novelty_w_spectral: f32,
    /// Novelty weight for energy flux.
    pub novelty_w_energy: f32,
    /// Novelty weight for HFC.
    pub novelty_w_hfc: f32,
    /// Novelty conditioning windows.
    pub novelty_local_mean_window: usize,
    /// Novelty moving-average smoothing window (frames). Use 0/1 to disable.
    pub novelty_smooth_window: usize,
    /// Optional: emit debug prints (stderr) for this track ID when running multi-resolution fusion.
    pub debug_track_id: Option<u32>,
    /// Optional: ground-truth BPM for debug scoring prints.
    pub debug_gt_bpm: Option<f32>,
    /// Optional: how many top candidates per hop to print in debug output.
    pub debug_top_n: usize,
    /// SuperFlux max-filter neighborhood radius (bins).
    pub superflux_max_filter_bins: usize,
}

/// Debug view of scored tempogram BPM candidates (validation/tuning only).
#[derive(Debug, Clone)]
pub struct TempogramCandidateDebug {
    /// Candidate tempo in BPM.
    pub bpm: f32,
    /// Combined score used for ranking (blended FFT+autocorr, with mild priors).
    pub score: f32,
    /// FFT-method normalized support in [0, 1].
    pub fft_norm: f32,
    /// Autocorrelation-method normalized support in [0, 1].
    pub autocorr_norm: f32,
    /// True if this candidate was selected as the final BPM.
    pub selected: bool,
}
/// Estimate BPM using tempogram approach (dual method: FFT + Autocorrelation)
///
/// This is the main entry point for tempogram-based BPM detection. It combines
/// both FFT and autocorrelation tempogram methods for maximum accuracy.
///
/// # Reference
///
/// Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
/// *IEEE Transactions on Audio, Speech, and Language Processing*.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
/// * `sample_rate` - Sample rate in Hz
/// * `hop_size` - Hop size used for STFT (samples per frame)
/// * `min_bpm` - Minimum BPM to consider (default: 40.0)
/// * `max_bpm` - Maximum BPM to consider (default: 240.0)
/// * `bpm_resolution` - BPM resolution for autocorrelation tempogram (default: 0.5)
///
/// # Returns
///
/// Best BPM estimate with confidence and method agreement
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Spectrogram is empty
/// - Invalid parameters
/// - Tempogram computation fails
///
/// # Strategy
///
/// 1. **Extract novelty curve**: Combine spectral flux, energy flux, and HFC
/// 2. **FFT tempogram**: Fast frequency-domain analysis (more consistent)
/// 3. **Autocorrelation tempogram**: Direct hypothesis testing (better resolution)
/// 4. **Selection logic**:
///    - If both agree (±2 BPM): Use average, boost confidence
///    - If FFT more confident: Use FFT (more consistent)
///    - If autocorr more confident: Use autocorr (better resolution)
///    - If disagree significantly: Flag for review, use ensemble
pub fn estimate_bpm_tempogram(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
) -> Result<BpmEstimate, AnalysisError> {
    Ok(
        estimate_bpm_tempogram_impl(
            magnitude_spec_frames,
            sample_rate,
            hop_size,
            min_bpm,
            max_bpm,
            bpm_resolution,
            None,
        )?
        .0,
    )
}

/// Estimate BPM using tempogram approach with **band fusion** enabled.
pub fn estimate_bpm_tempogram_band_fusion(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    band_cfg: TempogramBandFusionConfig,
) -> Result<BpmEstimate, AnalysisError> {
    Ok(
        estimate_bpm_tempogram_impl(
            magnitude_spec_frames,
            sample_rate,
            hop_size,
            min_bpm,
            max_bpm,
            bpm_resolution,
            Some(band_cfg),
        )?
        .0,
    )
}

/// Estimate BPM using tempogram analysis and return top-N candidates for diagnostics.
pub fn estimate_bpm_tempogram_with_candidates(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    top_n: usize,
) -> Result<(BpmEstimate, Vec<TempogramCandidateDebug>), AnalysisError> {
    let (est, mut cands) = estimate_bpm_tempogram_impl(
        magnitude_spec_frames,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
        bpm_resolution,
        None,
    )?;
    if top_n == 0 {
        cands.clear();
    } else if cands.len() > top_n {
        cands.truncate(top_n);
    }
    Ok((est, cands))
}

/// Estimate BPM using tempogram analysis with optional **multi-band novelty fusion**.
pub fn estimate_bpm_tempogram_with_candidates_band_fusion(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    top_n: usize,
    band_cfg: TempogramBandFusionConfig,
) -> Result<(BpmEstimate, Vec<TempogramCandidateDebug>), AnalysisError> {
    let (est, mut cands) = estimate_bpm_tempogram_impl(
        magnitude_spec_frames,
        sample_rate,
        hop_size,
        min_bpm,
        max_bpm,
        bpm_resolution,
        Some(band_cfg),
    )?;
    if top_n == 0 {
        cands.clear();
    } else if cands.len() > top_n {
        cands.truncate(top_n);
    }
    Ok((est, cands))
}

// --- implementation ---

fn estimate_bpm_tempogram_impl(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: u32,
    min_bpm: f32,
    max_bpm: f32,
    bpm_resolution: f32,
    band_cfg: Option<TempogramBandFusionConfig>,
) -> Result<(BpmEstimate, Vec<TempogramCandidateDebug>), AnalysisError> {
    log::debug!("Estimating BPM using tempogram: {} frames, sample_rate={}, hop_size={}, BPM range=[{:.1}, {:.1}]",
                magnitude_spec_frames.len(), sample_rate, hop_size, min_bpm, max_bpm);
    
    // --- Step 1: novelty extraction (full-band + optional multi-band) ---
    let n_bins = magnitude_spec_frames
        .first()
        .map(|f| f.len())
        .unwrap_or(0);
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }

    // Infer FFT size from the STFT magnitude width (compute_stft returns frame_size/2 + 1).
    let fft_size = (n_bins.saturating_sub(1)).saturating_mul(2).max(2);
    let freq_resolution = sample_rate as f32 / fft_size as f32;

    fn hz_to_bin(freq_hz: f32, freq_resolution: f32, n_bins: usize) -> usize {
        if !(freq_hz.is_finite() && freq_hz > 0.0) || !(freq_resolution.is_finite() && freq_resolution > 0.0) {
            return 0;
        }
        let b = (freq_hz / freq_resolution).round() as isize;
        b.clamp(0, (n_bins as isize).saturating_sub(1)) as usize
    }

    #[derive(Clone)]
    struct Variant {
        name: &'static str,
        w: f32,
        fft: Vec<(f32, f32)>,
        autocorr: Vec<(f32, f32)>,
        max_fft: f32,
        max_autocorr: f32,
    }

    // Full-band novelty (baseline).
    let sf_k = band_cfg
        .as_ref()
        .map(|c| c.superflux_max_filter_bins)
        .unwrap_or(4);
    let spectral_full = superflux_novelty(magnitude_spec_frames, sf_k)?;
    let energy_full = energy_flux_novelty(magnitude_spec_frames)?;
    let hfc_full = hfc_novelty(magnitude_spec_frames, sample_rate)?;
    let novelty_full = if let Some(cfg) = band_cfg.as_ref() {
        combined_novelty_with_params(
            &spectral_full,
            &energy_full,
            &hfc_full,
            cfg.novelty_w_spectral,
            cfg.novelty_w_energy,
            cfg.novelty_w_hfc,
            cfg.novelty_local_mean_window,
            cfg.novelty_smooth_window,
        )
    } else {
        combined_novelty(&spectral_full, &energy_full, &hfc_full)
    };
    if novelty_full.is_empty() {
        return Err(AnalysisError::ProcessingError(
            "Novelty curve is empty after extraction".to_string(),
        ));
    }

    let fft_full = fft_tempogram(&novelty_full, sample_rate, hop_size, min_bpm, max_bpm)?;
    let autocorr_full =
        autocorrelation_tempogram(&novelty_full, sample_rate, hop_size, min_bpm, max_bpm, bpm_resolution)?;

    let fft_best = find_best_bpm_fft(&fft_full);
    let autocorr_best = find_best_bpm_autocorr(&autocorr_full);

    let mut seed_variants: Vec<Variant> = Vec::new();
    seed_variants.push(Variant {
        name: "full",
        w: band_cfg.as_ref().map(|c| c.w_full).unwrap_or(1.0),
        max_fft: fft_full.first().map(|(_, p)| *p).unwrap_or(1.0).max(1e-12),
        max_autocorr: autocorr_full.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-12),
        fft: fft_full,
        autocorr: autocorr_full,
    });

    if let Some(cfg) = band_cfg.as_ref() {
        if cfg.enabled {
            // Define 3 musically motivated bands:
            // - low:   (≈kick/bass fundamentals)        up to low_max_hz
            // - mid:   (≈snare/body/rhythm textures)    low_max_hz..mid_max_hz
            // - high:  (≈hi-hats/attacks/transients)    mid_max_hz..high_max_hz/Nyquist
            //
            // Start at bin 1 to avoid DC.
            let b0 = 1usize.min(n_bins.saturating_sub(1));
            let b_low = hz_to_bin(cfg.low_max_hz, freq_resolution, n_bins).max(b0);
            let b_mid = hz_to_bin(cfg.mid_max_hz, freq_resolution, n_bins).max(b_low + 1);
            let b_hi = if cfg.high_max_hz > 0.0 {
                hz_to_bin(cfg.high_max_hz, freq_resolution, n_bins).max(b_mid + 1)
            } else {
                n_bins
            };
            let b_hi = b_hi.min(n_bins);

            let bands: [(&'static str, usize, usize, f32); 3] = [
                ("low", b0, b_low, cfg.w_low),
                ("mid", b_low, b_mid, cfg.w_mid),
                ("high", b_mid, b_hi, cfg.w_high),
            ];

            for (name, start, end, w) in bands {
                if !(w.is_finite() && w > 0.0) {
                    continue;
                }
                if end <= start + 1 {
                    continue;
                }

                let spectral = superflux_novelty_band(magnitude_spec_frames, sf_k, start, end)?;
                let energy = energy_flux_novelty_band(magnitude_spec_frames, start, end)?;
                let hfc = hfc_novelty_band(magnitude_spec_frames, start, end)?;
                let novelty = if let Some(cfg) = band_cfg.as_ref() {
                    combined_novelty_with_params(
                        &spectral,
                        &energy,
                        &hfc,
                        cfg.novelty_w_spectral,
                        cfg.novelty_w_energy,
                        cfg.novelty_w_hfc,
                        cfg.novelty_local_mean_window,
                        cfg.novelty_smooth_window,
                    )
                } else {
                    combined_novelty(&spectral, &energy, &hfc)
                };
                if novelty.is_empty() {
                    continue;
                }

                let fft = fft_tempogram(&novelty, sample_rate, hop_size, min_bpm, max_bpm)?;
                let autocorr =
                    autocorrelation_tempogram(&novelty, sample_rate, hop_size, min_bpm, max_bpm, bpm_resolution)?;

                seed_variants.push(Variant {
                    name,
                    w,
                    max_fft: fft.first().map(|(_, p)| *p).unwrap_or(1.0).max(1e-12),
                    max_autocorr: autocorr.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-12),
                    fft,
                    autocorr,
                });
            }
        }
    }

    // Additional representation: log-mel novelty tempogram.
    if let Some(cfg) = band_cfg.as_ref() {
        if cfg.enable_mel {
            let mel_curve = mel_superflux_novelty(
                magnitude_spec_frames,
                sample_rate,
                cfg.mel_n_mels,
                cfg.mel_fmin_hz,
                cfg.mel_fmax_hz,
                cfg.mel_max_filter_bins,
            )?;
            if !mel_curve.is_empty() {
                let fft = fft_tempogram(&mel_curve, sample_rate, hop_size, min_bpm, max_bpm)?;
                let autocorr =
                    autocorrelation_tempogram(&mel_curve, sample_rate, hop_size, min_bpm, max_bpm, bpm_resolution)?;
                seed_variants.push(Variant {
                    name: "mel",
                    w: cfg.w_mel,
                    max_fft: fft.first().map(|(_, p)| *p).unwrap_or(1.0).max(1e-12),
                    max_autocorr: autocorr.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-12),
                    fft,
                    autocorr,
                });
            }
        }
    }

    // Scoring variants:
    // - Always include full-band.
    // - If seed_only, bands are used ONLY for peak seeding (candidate generation), not scoring.
    let seed_only = band_cfg.as_ref().map(|c| c.seed_only).unwrap_or(true);
    let score_variants: Vec<Variant> = if seed_only {
        seed_variants
            .iter()
            .filter(|v| v.name == "full")
            .cloned()
            .collect()
    } else {
        seed_variants.clone()
    };

    let support_threshold = band_cfg
        .as_ref()
        .map(|c| c.support_threshold)
        .unwrap_or(0.25)
        .clamp(0.0, 1.0);
    let consensus_bonus = band_cfg
        .as_ref()
        .map(|c| c.consensus_bonus)
        .unwrap_or(0.0)
        .max(0.0);

    let w_sum: f32 = score_variants
        .iter()
        .map(|v| v.w.max(0.0))
        .sum::<f32>()
        .max(1e-6);
    
    // Step 4: Metrical-level selection (tempo folding) + scoring
    //
    // Empirically, the dominant failure mode is tempo octave / metrical-level selection (e.g., 2×).
    // Instead of preferring the higher BPM when harmonic relationships exist, we explicitly score
    // tempo-related candidates {T, T/2, 2T, T/3, 3T, 2T/3, 3T/2} using both tempograms and a
    // mild prior toward common DJ/tempo ranges (60–180 BPM) while preserving support for 40–240.
    let (fft_primary_bpm, fft_primary_conf) = fft_best
        .map(|r| (r.bpm, r.confidence))
        .unwrap_or((0.0, 0.0));
    let (autocorr_primary_bpm, autocorr_primary_conf) = autocorr_best
        .map(|r| (r.bpm, r.confidence))
        .unwrap_or((0.0, 0.0));

    // "Empty" guard: if all variants are empty in both representations, we failed.
    let all_empty = seed_variants.iter().all(|v| v.fft.is_empty() && v.autocorr.is_empty());
    if all_empty {
        return Err(AnalysisError::ProcessingError(
            "Both FFT and autocorrelation tempograms are empty".to_string(),
        ));
    }

    fn lookup_nearest(tempogram: &[(f32, f32)], bpm: f32, tol_bpm: f32) -> f32 {
        let mut best_d = f32::INFINITY;
        let mut best_v = 0.0f32;
        for (b, v) in tempogram.iter() {
            let d = (*b - bpm).abs();
            if d <= tol_bpm && d < best_d {
                best_d = d;
                best_v = *v;
            }
        }
        best_v
    }

    fn push_candidate(cands: &mut Vec<f32>, bpm: f32, min_bpm: f32, max_bpm: f32) {
        if bpm.is_finite() && bpm >= min_bpm && bpm <= max_bpm {
            cands.push(bpm);
        }
    }

    // Seed candidates: top-N peaks from both methods + primary picks
    let mut seed_bpms: Vec<f32> = Vec::new();
    for v in &seed_variants {
        seed_bpms.extend(v.fft.iter().take(8).map(|(b, _)| *b));
        seed_bpms.extend(v.autocorr.iter().take(8).map(|(b, _)| *b));
    }
    if fft_primary_bpm > 0.0 {
        seed_bpms.push(fft_primary_bpm);
    }
    if autocorr_primary_bpm > 0.0 {
        seed_bpms.push(autocorr_primary_bpm);
    }

    // Tempo folding factors to resolve metrical-level ambiguity
    const FACTORS: [f32; 7] = [1.0, 0.5, 2.0, 1.0 / 3.0, 3.0, 2.0 / 3.0, 3.0 / 2.0];

    let mut candidates: Vec<f32> = Vec::new();
    for base in seed_bpms {
        for f in FACTORS {
            push_candidate(&mut candidates, base * f, min_bpm, max_bpm);
        }
    }

    // Deduplicate by clustering within ~0.75 BPM
    candidates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut uniq: Vec<f32> = Vec::new();
    for bpm in candidates {
        if let Some(last) = uniq.last().copied() {
            if (bpm - last).abs() < 0.75 {
                continue;
            }
        }
        uniq.push(bpm);
    }

    // Preferable (but not exclusive) tempo range.
    //
    // Note: FMA ground truth includes many >180 BPM tracks; keep the prior mild.
    const PREFERRED_MIN: f32 = 60.0;
    const PREFERRED_MAX: f32 = 180.0;

    #[derive(Debug, Clone)]
    struct ScoredCandidate {
        bpm: f32,
        score: f32,
        fft_norm: f32,
        autocorr_norm: f32,
    }

    let mut scored: Vec<ScoredCandidate> = Vec::with_capacity(uniq.len());
    for bpm in uniq {
        // Use a nearest-neighbor lookup (not max-over-window) to avoid score plateaus where
        // adjacent BPMs all inherit the same peak strength (which collapses confidence to 0.0).
        let mut fft_acc = 0.0f32;
        let mut autocorr_acc = 0.0f32;
        for v in &score_variants {
            if v.w <= 0.0 {
                continue;
            }
            let fft_val = lookup_nearest(&v.fft, bpm, 0.75);
            let autocorr_val = lookup_nearest(&v.autocorr, bpm, bpm_resolution.max(0.5));
            fft_acc += v.w * (fft_val / v.max_fft).clamp(0.0, 1.0);
            autocorr_acc += v.w * (autocorr_val / v.max_autocorr).clamp(0.0, 1.0);
        }
        let fft_norm = (fft_acc / w_sum).clamp(0.0, 1.0);
        let autocorr_norm = (autocorr_acc / w_sum).clamp(0.0, 1.0);

        // Weighted combination:
        // Empirically, the FFT tempogram peak is often a stronger global indicator, while
        // autocorrelation can be broad/ambiguous for dense novelty curves. We therefore
        // bias the combination toward FFT while still incorporating autocorr support.
        // Weighted combination:
        // Autocorr gives finer BPM resolution and tends to be strong on rhythmic material,
        // while FFT provides global robustness. Use a balanced blend.
        let mut score = 0.55 * autocorr_norm + 0.45 * fft_norm;

        // Band-consensus bonus: if multiple bands (low/mid/high) support this BPM, lightly boost it.
        // This is intentionally *not* the same as letting bands directly drive the score.
        if consensus_bonus > 0.0
            && band_cfg
                .as_ref()
                .map(|c| c.enabled || c.enable_mel)
                .unwrap_or(false)
        {
            let mut support_bands = 0u32;
            for v in &seed_variants {
                if v.name == "full" {
                    continue;
                }
                let s_fft = (lookup_nearest(&v.fft, bpm, 0.75) / v.max_fft).clamp(0.0, 1.0);
                let s_ac = (lookup_nearest(&v.autocorr, bpm, bpm_resolution.max(0.5)) / v.max_autocorr)
                    .clamp(0.0, 1.0);
                let s = s_fft.max(s_ac);
                if s >= support_threshold {
                    support_bands += 1;
                }
            }
            if support_bands >= 2 {
                score *= 1.0 + consensus_bonus * (support_bands as f32 - 1.0);
            }
        }

        // Mild metrical prior: penalize extreme tempos unless strongly supported
        if bpm > PREFERRED_MAX {
            score *= 0.80;
        } else if bpm < PREFERRED_MIN {
            score *= 0.90;
        }

        scored.push(ScoredCandidate {
            bpm,
            score,
            fft_norm,
            autocorr_norm,
        });
    }

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut best = scored.first().cloned().ok_or_else(|| {
        AnalysisError::ProcessingError("No BPM candidates could be scored".to_string())
    })?;

    // Tempo-octave correction (DJ-centric metrical-level selection):
    // If the best candidate is above the preferred range (>180), check whether its /2 fold
    // is competitively scored. If so, prefer the folded value.
    //
    // This directly targets the dominant empirical error mode: ~2× predictions.
    if best.bpm > PREFERRED_MAX {
        let folded = best.bpm / 2.0;
        if folded >= min_bpm && folded <= max_bpm {
            if let Some(folded_scored) = scored
                .iter()
                .find(|c| (c.bpm - folded).abs() < 0.75)
                .cloned()
            {
                // Default behavior: tempo-octave ambiguity is common above ~180.
                // Keep the high BPM only if it is substantially stronger in *both* representations.
                let eps = 1e-6;
                let autocorr_ratio =
                    (best.autocorr_norm + eps) / (folded_scored.autocorr_norm + eps);
                let fft_ratio = (best.fft_norm + eps) / (folded_scored.fft_norm + eps);

                // Fold unless high BPM is convincingly stronger.
                if !(autocorr_ratio > 2.0 && fft_ratio > 2.0) {
                    log::debug!(
                        "Tempo-octave fold applied: {:.2} -> {:.2} (scores: high={:.3}, half={:.3}, ratios: autocorr={:.2}, fft={:.2})",
                        best.bpm,
                        folded_scored.bpm,
                        best.score,
                        folded_scored.score,
                        autocorr_ratio,
                        fft_ratio
                    );
                    best = folded_scored;
                }
            }
        }
    }

    let second = scored.get(1).cloned();

    // Confidence from separation between best and second-best scores
    let confidence = if best.score > 1e-12 {
        let second_score = second.as_ref().map(|s| s.score).unwrap_or(0.0);
        ((best.score - second_score).max(0.0) / best.score).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Method agreement: count methods whose primary pick matches the chosen BPM
    let mut method_agreement = 0u32;
    if fft_primary_bpm > 0.0 && (fft_primary_bpm - best.bpm).abs() < 2.0 {
        method_agreement += 1;
    }
    if autocorr_primary_bpm > 0.0 && (autocorr_primary_bpm - best.bpm).abs() < 2.0 {
        method_agreement += 1;
    }

    log::debug!(
        "Tempogram metrical selection: chosen {:.2} BPM (score={:.3}, conf={:.3}, fft_norm={:.3}, autocorr_norm={:.3}), primary FFT={:.2} (conf={:.3}), primary Autocorr={:.2} (conf={:.3}), variants={}",
        best.bpm,
        best.score,
        confidence,
        best.fft_norm,
        best.autocorr_norm,
        fft_primary_bpm,
        fft_primary_conf,
        autocorr_primary_bpm,
        autocorr_primary_conf,
        format!(
            "seed=[{}], score=[{}]",
            seed_variants.iter().map(|v| v.name).collect::<Vec<_>>().join("+"),
            score_variants.iter().map(|v| v.name).collect::<Vec<_>>().join("+")
        )
    );
    if let Some(s) = second {
        log::debug!(
            "Runner-up {:.2} BPM (score={:.3}, fft_norm={:.3}, autocorr_norm={:.3})",
            s.bpm,
            s.score,
            s.fft_norm,
            s.autocorr_norm
        );
    }
    for (i, c) in scored.iter().take(5).enumerate() {
        log::debug!(
            "Top {}: {:.2} BPM (score={:.3}, fft_norm={:.3}, autocorr_norm={:.3})",
            i + 1,
            c.bpm,
            c.score,
            c.fft_norm,
            c.autocorr_norm
        );
    }

    let estimate = BpmEstimate {
        bpm: best.bpm,
        confidence,
        method_agreement,
    };

    let diagnostics: Vec<TempogramCandidateDebug> = scored
        .iter()
        .map(|c| TempogramCandidateDebug {
            bpm: c.bpm,
            score: c.score,
            fft_norm: c.fft_norm,
            autocorr_norm: c.autocorr_norm,
            selected: (c.bpm - best.bpm).abs() < 0.75,
        })
        .collect();

    Ok((estimate, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_estimate_bpm_tempogram_basic() {
        // Create spectrogram with periodic pattern
        let mut spectrogram = vec![vec![0.1f32; 1024]; 500];
        
        // Add periodic pattern (simulating 120 BPM)
        let period = 43; // Approximate for 120 BPM at 44.1kHz, 512 hop
        for i in 0..spectrogram.len() {
            if i % period == 0 {
                for bin in 0..512 {
                    spectrogram[i][bin] = 1.0;
                }
            }
        }
        
        let result = estimate_bpm_tempogram(&spectrogram, 44100, 512, 100.0, 140.0, 0.5).unwrap();
        
        // Should find BPM around 120
        assert!(result.bpm >= 115.0 && result.bpm <= 125.0,
                "Expected BPM around 120, got {:.1}", result.bpm);
        assert!(result.confidence >= 0.0 && result.confidence <= 1.0);
    }
    
    #[test]
    fn test_estimate_bpm_tempogram_empty() {
        let spectrogram = vec![];
        let result = estimate_bpm_tempogram(&spectrogram, 44100, 512, 40.0, 240.0, 0.5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_estimate_bpm_tempogram_agreement() {
        // This test would require mocking the tempogram methods to ensure agreement
        // For now, we test that the function runs without error
        let spectrogram = vec![vec![0.5f32; 1024]; 200];
        let result = estimate_bpm_tempogram(&spectrogram, 44100, 512, 40.0, 240.0, 0.5);
        
        // Should either succeed or fail gracefully
        match result {
            Ok(est) => {
                assert!(est.bpm >= 40.0 && est.bpm <= 240.0);
                assert!(est.confidence >= 0.0 && est.confidence <= 1.0);
            }
            Err(_) => {
                // Failure is acceptable for random input
            }
        }
    }
}

