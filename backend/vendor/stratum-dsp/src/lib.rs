//! # Stratum DSP
//!
//! A professional-grade audio analysis engine for DJ applications, providing
//! accurate BPM detection, key detection, and beat tracking.
//!
//! ## Features
//!
//! - **BPM Detection**: Multi-method onset detection with autocorrelation and comb filterbank
//! - **Key Detection**: Chroma-based analysis with Krumhansl-Kessler template matching
//! - **Beat Tracking**: HMM-based beat grid generation with tempo drift correction
//! - **ML Refinement**: Optional ONNX model for edge case correction (Phase 2)
//!
//! ## Quick Start
//!
//! ```no_run
//! use stratum_dsp::{analyze_audio, AnalysisConfig};
//!
//! // Load audio samples (mono, f32, normalized)
//! let samples: Vec<f32> = vec![]; // Your audio data
//! let sample_rate = 44100;
//!
//! // Analyze
//! let result = analyze_audio(&samples, sample_rate, AnalysisConfig::default())?;
//!
//! println!("BPM: {:.2} (confidence: {:.2})", result.bpm, result.bpm_confidence);
//! println!("Key: {:?} (confidence: {:.2})", result.key, result.key_confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```
//!
//! ## Architecture
//!
//! The analysis pipeline follows this flow:
//!
//! ```text
//! Audio Input -> Preprocessing -> Feature Extraction -> Analysis -> ML Refinement -> Output
//! ```
//!
//! See the [module documentation](https://docs.rs/stratum-dsp) for details.

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod analysis;
pub mod config;
pub mod error;
pub mod features;
pub mod preprocessing;

#[cfg(feature = "ml")]
pub mod ml;

// Re-export main types
pub use analysis::result::{AnalysisResult, AnalysisMetadata, BeatGrid, Key, KeyType};
pub use analysis::confidence::{AnalysisConfidence, compute_confidence};
pub use config::AnalysisConfig;
pub use error::AnalysisError;

/// Main analysis function
///
/// Analyzes audio samples and returns comprehensive analysis results including
/// BPM, key, beat grid, and confidence scores.
///
/// # Arguments
///
/// * `samples` - Mono audio samples, normalized to [-1.0, 1.0]
/// * `sample_rate` - Sample rate in Hz (typically 44100 or 48000)
/// * `config` - Analysis configuration parameters
///
/// # Returns
///
/// `AnalysisResult` containing BPM, key, beat grid, and confidence metrics
///
/// # Errors
///
/// Returns `AnalysisError` if analysis fails (invalid input, processing error, etc.)
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::{analyze_audio, AnalysisConfig};
///
/// let samples = vec![0.0f32; 44100 * 30]; // 30 seconds of silence
/// let result = analyze_audio(&samples, 44100, AnalysisConfig::default())?;
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn analyze_audio(
    samples: &[f32],
    sample_rate: u32,
    config: AnalysisConfig,
) -> Result<AnalysisResult, AnalysisError> {
    analyze_parts(samples, sample_rate, config, true, true)
}

/// BPM and beat grid only (vendored patch): `analyze_audio` without its key
/// step. The BPM, its confidence and the beat grid are the ones
/// `analyze_audio` gives for the same input; the key is `Key::Major(0)` with
/// zero confidence and clarity.
pub fn analyze_bpm(
    samples: &[f32],
    sample_rate: u32,
    config: AnalysisConfig,
) -> Result<AnalysisResult, AnalysisError> {
    analyze_parts(samples, sample_rate, config, true, false)
}

/// Key only (vendored patch): `analyze_audio` without its onset, tempo and
/// beat-tracking steps. The key, its confidence and clarity are the ones
/// `analyze_audio` gives for the same input, unless
/// `enable_key_beat_synchronous` is on (off by default): that mode needs the
/// beat grid, and without one the key comes from the plain chroma path. The
/// BPM is 0 and the beat grid empty.
pub fn analyze_key(
    samples: &[f32],
    sample_rate: u32,
    config: AnalysisConfig,
) -> Result<AnalysisResult, AnalysisError> {
    analyze_parts(samples, sample_rate, config, false, true)
}

/// The analysis pipeline; `detect_bpm` / `detect_key` skip the steps only
/// the other result needs (vendored patch). Both on is upstream's
/// `analyze_audio`, step for step.
fn analyze_parts(
    samples: &[f32],
    sample_rate: u32,
    config: AnalysisConfig,
    detect_bpm: bool,
    detect_key: bool,
) -> Result<AnalysisResult, AnalysisError> {
    use std::time::Instant;
    let start_time = Instant::now();
    
    log::debug!("Starting audio analysis: {} samples at {} Hz", samples.len(), sample_rate);
    
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Invalid sample rate".to_string()));
    }
    
    // Phase 1A: Preprocessing
    let mut processed_samples = samples.to_vec();
    
    // 1. Normalization
    use preprocessing::normalization::{normalize, NormalizationConfig};
    if config.enable_normalization {
        let norm_config = NormalizationConfig {
            method: config.normalization,
            target_loudness_lufs: -14.0, // Default target
            max_headroom_db: 1.0,
        };
        let _loudness_metadata = normalize(&mut processed_samples, norm_config, sample_rate as f32)?;
    } else {
        log::debug!("Skipping normalization (enable_normalization=false)");
    }
    
    // 2. Silence detection and trimming
    use preprocessing::silence::{detect_and_trim, SilenceDetector};
    let (trimmed_samples, _silence_regions) = if config.enable_silence_trimming {
        let silence_detector = SilenceDetector {
            threshold_db: config.min_amplitude_db,
            min_duration_ms: 500,
            frame_size: config.frame_size,
        };
        detect_and_trim(&processed_samples, sample_rate, silence_detector)?
    } else {
        log::debug!("Skipping silence trimming (enable_silence_trimming=false)");
        (processed_samples.clone(), Vec::new())
    };
    
    if trimmed_samples.is_empty() {
        return Err(AnalysisError::ProcessingError("Audio is entirely silent after trimming".to_string()));
    }
    
    // Phase 1A: Onset Detection
    // Note: For now, we only have energy flux working directly on samples
    // Spectral methods (spectral_flux, hfc, hpss) require STFT which will be added in Phase 1B
    use features::onset::energy_flux::detect_energy_flux_onsets;
    
    let energy_onsets = if detect_bpm {
        detect_energy_flux_onsets(
            &trimmed_samples,
            config.frame_size,
            config.hop_size,
            -20.0, // threshold_db
        )?
    } else {
        Vec::new()
    };
    
    log::debug!("Detected {} onsets using energy flux", energy_onsets.len());
    
    // Phase 1F: Tempogram-based BPM Detection (replaces Phase 1B period estimation)
    // Compute STFT once (used by tempogram BPM and STFT-based onset detectors)
    use features::chroma::extractor::compute_stft;
    let magnitude_spec_frames = compute_stft(
        &trimmed_samples,
        config.frame_size,
        config.hop_size,
    )?;

    // Onset consensus (improves beat tracking + legacy BPM fallback robustness)
    //
    // Important: Tempogram BPM does NOT use these onsets, but the legacy BPM estimator and
    // beat tracker do. Since we will compare / integrate legacy + tempogram estimates,
    // we want the best onsets we can get.
    let mut onsets_for_legacy: Vec<usize> = energy_onsets.clone();
    let mut onsets_for_beat_tracking: Vec<usize> = energy_onsets.clone();

    if detect_bpm && config.enable_onset_consensus && !magnitude_spec_frames.is_empty() {
        use features::onset::consensus::{vote_onsets, OnsetConsensus};
        use features::onset::hfc::detect_hfc_onsets;
        use features::onset::spectral_flux::detect_spectral_flux_onsets;

        let to_samples = |frames: Vec<usize>, hop_size: usize, n_samples: usize| -> Vec<usize> {
            let mut out: Vec<usize> = frames
                .into_iter()
                .map(|f| f.saturating_mul(hop_size))
                .filter(|&s| s < n_samples)
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        };

        let spectral_onsets_frames = match detect_spectral_flux_onsets(
            &magnitude_spec_frames,
            config.onset_threshold_percentile,
        ) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Spectral flux onset detection failed: {}", e);
                Vec::new()
            }
        };
        let spectral_onsets_samples = to_samples(
            spectral_onsets_frames,
            config.hop_size,
            trimmed_samples.len(),
        );

        let hfc_onsets_frames = match detect_hfc_onsets(
            &magnitude_spec_frames,
            sample_rate,
            config.onset_threshold_percentile,
        ) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("HFC onset detection failed: {}", e);
                Vec::new()
            }
        };
        let hfc_onsets_samples = to_samples(hfc_onsets_frames, config.hop_size, trimmed_samples.len());

        let hpss_onsets_samples = if config.enable_hpss_onsets {
            use features::onset::hpss::{detect_hpss_onsets, hpss_decompose};
            match hpss_decompose(&magnitude_spec_frames, config.hpss_margin)
                .and_then(|(_, p)| detect_hpss_onsets(&p, config.onset_threshold_percentile))
            {
                Ok(hpss_frames) => to_samples(hpss_frames, config.hop_size, trimmed_samples.len()),
                Err(e) => {
                    log::warn!("HPSS onset detection failed: {}", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        log::debug!(
            "Onset detectors: energy_flux(samples)={}, spectral_flux(samples)={}, hfc(samples)={}, hpss(samples)={}",
            energy_onsets.len(),
            spectral_onsets_samples.len(),
            hfc_onsets_samples.len(),
            hpss_onsets_samples.len()
        );

        let consensus = OnsetConsensus {
            energy_flux: energy_onsets.clone(),
            spectral_flux: spectral_onsets_samples,
            hfc: hfc_onsets_samples,
            hpss: hpss_onsets_samples,
        };

        match vote_onsets(
            consensus,
            config.onset_consensus_weights,
            config.onset_consensus_tolerance_ms,
            sample_rate,
        ) {
            Ok(candidates) => {
                // Default policy: prefer onsets confirmed by >=2 methods.
                // If that yields nothing, fall back to the full clustered set (>=1 method).
                let mut strong: Vec<usize> = candidates
                    .iter()
                    .filter(|c| c.voted_by >= 2)
                    .map(|c| c.time_samples)
                    .collect();
                strong.sort_unstable();
                strong.dedup();

                let mut any: Vec<usize> = candidates.iter().map(|c| c.time_samples).collect();
                any.sort_unstable();
                any.dedup();

                let chosen = if !strong.is_empty() { strong } else { any };
                if !chosen.is_empty() {
                    log::debug!(
                        "Onset consensus: chosen {} onsets (strong>=2 methods: {}, total_clusters: {})",
                        chosen.len(),
                        candidates.iter().filter(|c| c.voted_by >= 2).count(),
                        candidates.len()
                    );
                    onsets_for_legacy = chosen.clone();
                    onsets_for_beat_tracking = chosen;
                } else {
                    log::debug!("Onset consensus produced no candidates; using energy-flux onsets");
                }
            }
            Err(e) => {
                log::warn!("Onset consensus voting failed: {}", e);
            }
        }
    }
    
    // BPM estimation: tempogram (Phase 1F) + legacy (Phase 1B), optionally fused
    let legacy_estimate = {
        use features::period::{estimate_bpm, estimate_bpm_with_guardrails, LegacyBpmGuardrails};
        if onsets_for_legacy.len() >= 2 {
            if config.enable_legacy_bpm_guardrails {
                let guardrails = LegacyBpmGuardrails {
                    preferred_min: config.legacy_bpm_preferred_min,
                    preferred_max: config.legacy_bpm_preferred_max,
                    soft_min: config.legacy_bpm_soft_min,
                    soft_max: config.legacy_bpm_soft_max,
                    mul_preferred: config.legacy_bpm_conf_mul_preferred,
                    mul_soft: config.legacy_bpm_conf_mul_soft,
                    mul_extreme: config.legacy_bpm_conf_mul_extreme,
                };
                estimate_bpm_with_guardrails(
                    &onsets_for_legacy,
                    sample_rate,
                    config.hop_size,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    guardrails,
                )?
            } else {
                estimate_bpm(
                    &onsets_for_legacy,
                    sample_rate,
                    config.hop_size,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                )?
            }
        } else {
            None
        }
    };

    let mut tempogram_candidates: Option<Vec<crate::analysis::result::TempoCandidateDebug>> = None;
    let mut tempogram_multi_res_triggered: Option<bool> = None;
    let mut tempogram_multi_res_used: Option<bool> = None;
    let mut tempogram_percussive_triggered: Option<bool> = None;
    let mut tempogram_percussive_used: Option<bool> = None;

    let tempogram_estimate = if detect_bpm && !config.force_legacy_bpm && !magnitude_spec_frames.is_empty() {
        use crate::analysis::result::TempoCandidateDebug;
        use features::period::multi_resolution::multi_resolution_tempogram_from_samples;
        use features::period::tempogram::{
            estimate_bpm_tempogram,
            estimate_bpm_tempogram_band_fusion,
            estimate_bpm_tempogram_with_candidates,
            estimate_bpm_tempogram_with_candidates_band_fusion,
            TempogramBandFusionConfig,
        };

        let band_cfg = TempogramBandFusionConfig {
            enabled: config.enable_tempogram_band_fusion,
            low_max_hz: config.tempogram_band_low_max_hz,
            mid_max_hz: config.tempogram_band_mid_max_hz,
            high_max_hz: config.tempogram_band_high_max_hz,
            w_full: config.tempogram_band_w_full,
            w_low: config.tempogram_band_w_low,
            w_mid: config.tempogram_band_w_mid,
            w_high: config.tempogram_band_w_high,
            seed_only: config.tempogram_band_seed_only,
            support_threshold: config.tempogram_band_support_threshold,
            consensus_bonus: config.tempogram_band_consensus_bonus,
            enable_mel: config.enable_tempogram_mel_novelty,
            mel_n_mels: config.tempogram_mel_n_mels,
            mel_fmin_hz: config.tempogram_mel_fmin_hz,
            mel_fmax_hz: config.tempogram_mel_fmax_hz,
            mel_max_filter_bins: config.tempogram_mel_max_filter_bins,
            w_mel: config.tempogram_mel_weight,
            novelty_w_spectral: config.tempogram_novelty_w_spectral,
            novelty_w_energy: config.tempogram_novelty_w_energy,
            novelty_w_hfc: config.tempogram_novelty_w_hfc,
            novelty_local_mean_window: config.tempogram_novelty_local_mean_window,
            novelty_smooth_window: config.tempogram_novelty_smooth_window,
            debug_track_id: config.debug_track_id,
            debug_gt_bpm: config.debug_gt_bpm,
            debug_top_n: config.debug_top_n,
            superflux_max_filter_bins: config.tempogram_superflux_max_filter_bins,
        };

        let use_aux_variants = config.enable_tempogram_band_fusion
            || config.enable_tempogram_mel_novelty
            || config.tempogram_band_consensus_bonus > 0.0;

        if config.enable_tempogram_multi_resolution {
            // Run single-resolution tempogram first; only escalate to multi-resolution
            // when the result looks ambiguous (prevents global regressions).
            let base_top_n = config
                .tempogram_candidates_top_n
                .max(config.tempogram_multi_res_top_k)
                .max(10);

            let base_call = if use_aux_variants {
                estimate_bpm_tempogram_with_candidates_band_fusion(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    base_top_n,
                    band_cfg.clone(),
                )
            } else {
                estimate_bpm_tempogram_with_candidates(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    base_top_n,
                )
            };

            match base_call {
                Ok((base_est, base_cands)) => {
                    let trap_low = base_est.bpm >= 55.0 && base_est.bpm <= 80.0;
                    let trap_high = base_est.bpm >= 170.0 && base_est.bpm <= 200.0;

                    // Additional ambiguity detection: if the single-resolution candidate list already
                    // contains strong tempo-family alternatives (2× or 1/2×), escalate to multi-res.
                    //
                    // This catches cases like GT~184 predicted ~92 where base BPM is not in our
                    // original “trap” window but is a classic half-time error.
                    fn cand_support(
                        cands: &[features::period::tempogram::TempogramCandidateDebug],
                        bpm: f32,
                        tol: f32,
                    ) -> f32 {
                        let mut best = 0.0f32;
                        for c in cands {
                            if (c.bpm - bpm).abs() <= tol {
                                best = best.max(c.score);
                            }
                        }
                        best
                    }

                    let tol = 2.0f32.max(config.bpm_resolution);
                    let s_base = cand_support(&base_cands, base_est.bpm, tol);
                    let s_2x = cand_support(&base_cands, base_est.bpm * 2.0, tol);
                    let s_half = cand_support(&base_cands, base_est.bpm * 0.5, tol);
                    let family_competes = (s_2x > 0.0 && s_2x >= s_base * 0.90)
                        || (s_half > 0.0 && s_half >= s_base * 0.90);

                    // IMPORTANT: Our tempogram "confidence" is currently conservative and can be low even
                    // for correct tempos. If we use a generic confidence threshold, we end up escalating
                    // on nearly every track (catastrophic for performance, especially with HPSS).
                    //
                    // For now, only escalate in the known tempo-family trap zones. We'll widen this later
                    // once we have a better uncertainty measure.
                    // Escape hatch: if confidence/agreement is poor and a 2× fold would land in the
                    // high trap zone (or a 1/2× fold would land in the low trap zone), escalate even
                    // if the candidate list didn’t surface it strongly (prevents missing half-time errors).
                    // Only use the "fold_into_trap" escape hatch for the missed half-time case:
                    // base ~90, true ~180. Do NOT trigger on base ~120 (since 120/2=60 is common and
                    // would cause unnecessary multi-res runs / regressions).
                    let fold_into_trap = base_est.bpm * 2.0 >= 170.0 && base_est.bpm * 2.0 <= 200.0;
                    let weak_base = base_est.method_agreement == 0 || base_est.confidence < 0.06;

                    let ambiguous = trap_low || trap_high || family_competes || (weak_base && fold_into_trap);
                    // Instrumentation: "triggered" means we *considered* escalation, not just "base looks ambiguous".
                    tempogram_multi_res_triggered = Some(ambiguous);

                    if let Some(track_id) = config.debug_track_id {
                        eprintln!("\n=== DEBUG base tempogram (track_id={}) ===", track_id);
                        if let Some(gt) = config.debug_gt_bpm {
                            eprintln!("GT bpm: {:.3}", gt);
                        }
                        eprintln!(
                            "base_est: bpm={:.2} conf={:.4} agree={} (trap_low={} trap_high={} ambiguous={})",
                            base_est.bpm,
                            base_est.confidence,
                            base_est.method_agreement,
                            trap_low,
                            trap_high,
                            ambiguous
                        );
                        eprintln!(
                            "ambiguity signals: family_competes={} (s_base={:.4} s_2x={:.4} s_half={:.4}) weak_base={} fold_into_trap={}",
                            family_competes,
                            s_base,
                            s_2x,
                            s_half,
                            weak_base,
                            fold_into_trap
                        );
                        if !ambiguous {
                            eprintln!("NOTE: multi-res not run (outside trap zones).");
                        }
                    }

                    let mut chosen_est = base_est.clone();
                    let mut chosen_cands = base_cands;
                    let mut used_mr = false;

                    if ambiguous {
                        match multi_resolution_tempogram_from_samples(
                            &trimmed_samples,
                            sample_rate,
                            config.frame_size,
                            config.min_bpm,
                            config.max_bpm,
                            config.bpm_resolution,
                            config.tempogram_multi_res_top_k,
                            config.tempogram_multi_res_w512,
                            config.tempogram_multi_res_w256,
                            config.tempogram_multi_res_w1024,
                            config.tempogram_multi_res_structural_discount,
                            config.tempogram_multi_res_double_time_512_factor,
                            config.tempogram_multi_res_margin_threshold,
                            config.tempogram_multi_res_use_human_prior,
                            Some(band_cfg.clone()),
                        ) {
                            Ok((mr_est, mr_cands_512)) => {
                                let mr_est_log = mr_est.clone();
                                // Choose multi-res only if it provides stronger evidence or
                                // a safer tempo-family choice in the trap regions.
                                let rel = if base_est.bpm > 1e-6 {
                                    (mr_est.bpm / base_est.bpm).max(base_est.bpm / mr_est.bpm)
                                } else {
                                    1.0
                                };
                                let family_related =
                                    (rel - 2.0).abs() < 0.05 || (rel - 1.5).abs() < 0.05 || (rel - (4.0 / 3.0)).abs() < 0.05;

                                // Hard safety rule: do not “promote” a sane in-range tempo into an extreme
                                // high tempo (e.g., 120 -> 240). Multi-res should primarily resolve
                                // octave *folding* errors, not create them.
                                let forbid_promote_high = base_est.bpm <= 180.0 && mr_est.bpm > 180.0;

                                let mr_better = !forbid_promote_high
                                    && (mr_est.confidence >= (base_est.confidence + 0.05)
                                        || (mr_est.method_agreement > base_est.method_agreement
                                            && mr_est.confidence >= base_est.confidence * 0.90)
                                        || ((trap_low || trap_high)
                                            && family_related
                                            && mr_est.confidence >= base_est.confidence * 0.88
                                            // Additional safety: only accept family moves that land in a
                                            // typical music/DJ tempo band unless base was already extreme.
                                            && ((mr_est.bpm >= 70.0 && mr_est.bpm <= 180.0) || base_est.bpm > 180.0)));

                                if mr_better {
                                    chosen_est = mr_est;
                                    chosen_cands = mr_cands_512;
                                    used_mr = true;
                                }

                                if let Some(track_id) = config.debug_track_id {
                                    eprintln!("\n=== DEBUG multi-res decision (track_id={}) ===", track_id);
                                    if let Some(gt) = config.debug_gt_bpm {
                                        eprintln!("GT bpm: {:.3}", gt);
                                    }
                                    eprintln!("base_est: bpm={:.2} conf={:.4} agree={}", base_est.bpm, base_est.confidence, base_est.method_agreement);
                                    eprintln!("mr_est:   bpm={:.2} conf={:.4} agree={}", mr_est_log.bpm, mr_est_log.confidence, mr_est_log.method_agreement);
                                    eprintln!("ambiguous(trap_low||trap_high)={}", ambiguous);
                                    eprintln!("rel={:.3} family_related={} forbid_promote_high={}", rel, family_related, forbid_promote_high);
                                    eprintln!("mr_better={} used_mr={}", mr_better, used_mr);
                                }
                            }
                            Err(e) => {
                                log::debug!("Multi-resolution escalation skipped (failed): {}", e);
                            }
                        }
                    }
                    tempogram_multi_res_used = Some(used_mr);

                    // Percussive-only fallback (HPSS) for ambiguous cases (generation improvement).
                    //
                    // Important: HPSS is expensive. We only run it when we are in the classic
                    // low-tempo ambiguity zone where sustained harmonic content commonly causes
                    // half/double-time traps.
                    let percussive_needed = ambiguous && trap_low;
                    tempogram_percussive_triggered = Some(percussive_needed);

                    if config.enable_tempogram_percussive_fallback && percussive_needed {
                        use features::onset::hpss::hpss_decompose;

                        // Decompose the already computed spectrogram at the base hop_size.
                        match hpss_decompose(&magnitude_spec_frames, config.hpss_margin) {
                            Ok((_h, p)) => {
                                // Re-run tempogram on percussive component.
                                let p_call = if use_aux_variants {
                                    estimate_bpm_tempogram_with_candidates_band_fusion(
                                        &p,
                                        sample_rate,
                                        config.hop_size as u32,
                                        config.min_bpm,
                                        config.max_bpm,
                                        config.bpm_resolution,
                                        base_top_n,
                                        band_cfg.clone(),
                                    )
                                } else {
                                    estimate_bpm_tempogram_with_candidates(
                                        &p,
                                        sample_rate,
                                        config.hop_size as u32,
                                        config.min_bpm,
                                        config.max_bpm,
                                        config.bpm_resolution,
                                        base_top_n,
                                    )
                                };

                                match p_call {
                                    Ok((p_est, p_cands)) => {
                                        // Accept percussive estimate only when it is a tempo-family move
                                        // and does not promote sane tempos into extremes.
                                        let rel = if chosen_est.bpm > 1e-6 {
                                            (p_est.bpm / chosen_est.bpm).max(chosen_est.bpm / p_est.bpm)
                                        } else {
                                            1.0
                                        };
                                        let family_related = (rel - 2.0).abs() < 0.05
                                            || (rel - 1.5).abs() < 0.05
                                            || (rel - (4.0 / 3.0)).abs() < 0.05
                                            || (rel - (3.0 / 2.0)).abs() < 0.05
                                            || (rel - (2.0 / 3.0)).abs() < 0.05
                                            || (rel - (3.0 / 4.0)).abs() < 0.05;

                                        let forbid_promote_high = chosen_est.bpm <= 180.0 && p_est.bpm > 180.0;

                                        // Slightly more permissive acceptance in the low-tempo trap region:
                                        // if percussive yields a coherent 2× tempo in a common range, take it
                                        // even if confidence is only marginally better.
                                        let base_low_trap = trap_low || base_est.bpm < 95.0;
                                        let percussive_in_common = p_est.bpm >= 70.0 && p_est.bpm <= 180.0;

                                        let p_better = !forbid_promote_high
                                            && family_related
                                            && percussive_in_common
                                            && (p_est.confidence >= chosen_est.confidence + 0.04
                                                || (base_low_trap && p_est.confidence >= chosen_est.confidence * 0.85)
                                                || (p_est.method_agreement > chosen_est.method_agreement
                                                    && p_est.confidence >= chosen_est.confidence * 0.92));

                                        if p_better {
                                            chosen_est = p_est;
                                            chosen_cands = p_cands;
                                            tempogram_percussive_used = Some(true);
                                        } else {
                                            tempogram_percussive_used = Some(false);
                                        }
                                    }
                                    Err(e) => {
                                        log::debug!("Percussive tempogram fallback failed: {}", e);
                                        tempogram_percussive_used = Some(false);
                                    }
                                }
                            }
                            Err(e) => {
                                log::debug!("HPSS decomposition for percussive tempogram failed: {}", e);
                                tempogram_percussive_used = Some(false);
                            }
                        }
                    } else if config.enable_tempogram_percussive_fallback {
                        tempogram_percussive_used = Some(false);
                    }

                    if config.emit_tempogram_candidates {
                        tempogram_candidates = Some(
                            chosen_cands
                                .into_iter()
                                .map(|c| TempoCandidateDebug {
                                    bpm: c.bpm,
                                    score: c.score,
                                    fft_norm: c.fft_norm,
                                    autocorr_norm: c.autocorr_norm,
                                    selected: c.selected,
                                })
                                .collect(),
                        );
                    }

                    log::debug!(
                        "Tempogram BPM estimate: {:.2} (confidence: {:.3}, method_agreement: {}, multi_res={})",
                        chosen_est.bpm,
                        chosen_est.confidence,
                        chosen_est.method_agreement,
                        ambiguous
                    );

                    Some(chosen_est)
                }
                Err(e) => {
                    log::warn!("Tempogram BPM detection failed: {}", e);
                    None
                }
            }
        } else if config.emit_tempogram_candidates {
            let call = if use_aux_variants {
                estimate_bpm_tempogram_with_candidates_band_fusion(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    config.tempogram_candidates_top_n,
                    band_cfg.clone(),
                )
            } else {
                estimate_bpm_tempogram_with_candidates(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    config.tempogram_candidates_top_n,
                )
            };

            match call {
                Ok((estimate, cands)) => {
                    tempogram_candidates = Some(
                        cands.into_iter()
                            .map(|c| TempoCandidateDebug {
                                bpm: c.bpm,
                                score: c.score,
                                fft_norm: c.fft_norm,
                                autocorr_norm: c.autocorr_norm,
                                selected: c.selected,
                            })
                            .collect(),
                    );
                    log::debug!(
                        "Tempogram BPM estimate: {:.2} (confidence: {:.3}, method_agreement: {}, candidates_emitted={})",
                        estimate.bpm,
                        estimate.confidence,
                        estimate.method_agreement,
                        tempogram_candidates.as_ref().map(|v| v.len()).unwrap_or(0)
                    );
                    Some(estimate)
                }
                Err(e) => {
                    log::warn!("Tempogram BPM detection failed: {}", e);
                    None
                }
            }
        } else {
            let call = if use_aux_variants {
                estimate_bpm_tempogram_band_fusion(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                    band_cfg.clone(),
                )
            } else {
                estimate_bpm_tempogram(
                    &magnitude_spec_frames,
                    sample_rate,
                    config.hop_size as u32,
                    config.min_bpm,
                    config.max_bpm,
                    config.bpm_resolution,
                )
            };

            match call {
                Ok(estimate) => {
                    log::debug!(
                        "Tempogram BPM estimate: {:.2} (confidence: {:.3}, method_agreement: {})",
                        estimate.bpm,
                        estimate.confidence,
                        estimate.method_agreement
                    );
                    Some(estimate)
                }
                Err(e) => {
                    log::warn!("Tempogram BPM detection failed: {}", e);
                    None
                }
            }
        }
    } else {
        if !detect_bpm {
            log::debug!("Skipping BPM detection (key only)");
        } else if config.force_legacy_bpm {
            log::debug!("Forcing legacy BPM estimation (force_legacy_bpm=true)");
        } else if magnitude_spec_frames.is_empty() {
            log::warn!("Could not compute STFT for tempogram");
        }
        None
    };

    let (bpm, bpm_confidence) = if config.force_legacy_bpm {
        legacy_estimate
            .as_ref()
            .map(|e| (e.bpm, e.confidence))
            .unwrap_or((0.0, 0.0))
    } else if config.enable_bpm_fusion {
        // Fusion (safe validator mode):
        // - **Never** override the tempogram BPM (so fusion cannot regress BPM accuracy).
        // - Use legacy only to adjust *confidence* and emit diagnostics.
        let (t_bpm, t_conf, t_agree) = tempogram_estimate
            .as_ref()
            .map(|e| (e.bpm, e.confidence, e.method_agreement))
            .unwrap_or((0.0, 0.0, 0));
        let (l_bpm, l_conf_raw) = legacy_estimate
            .as_ref()
            .map(|e| (e.bpm, e.confidence))
            .unwrap_or((0.0, 0.0));
        let l_conf = l_conf_raw.clamp(0.0, 1.0);

        // If tempogram is unavailable, fall back to legacy (guardrailed).
        if t_bpm <= 0.0 {
            legacy_estimate
                .as_ref()
                .map(|e| (e.bpm, e.confidence))
                .unwrap_or((0.0, 0.0))
        } else {
            let tol = 2.0f32;
            let mut conf = t_conf.clamp(0.0, 1.0);

            // Agreement / validation scoring between legacy and tempogram BPMs.
            let agreement = if l_bpm > 0.0 {
                // Allow common metrical ambiguity relations without forcing an override.
                let diffs = [
                    (l_bpm - t_bpm).abs(),
                    (l_bpm - (t_bpm * 0.5)).abs(),
                    (l_bpm - (t_bpm * 2.0)).abs(),
                    (l_bpm - (t_bpm * (2.0 / 3.0))).abs(),
                    (l_bpm - (t_bpm * (3.0 / 2.0))).abs(),
                ];
                diffs.into_iter().any(|d| d <= tol)
            } else {
                false
            };

            if agreement {
                // Modest boost when legacy is consistent (even if it’s at a different metrical level).
                let boost = 0.12 * l_conf;
                conf = (conf + boost).clamp(0.0, 1.0);
                log::debug!(
                    "BPM fusion (validator): tempogram {:.2} kept; legacy {:.2} validates (agree≈true); conf {:.3}->{:.3}; temp_agree={}",
                    t_bpm,
                    l_bpm,
                    t_conf,
                    conf,
                    t_agree
                );
            } else if l_bpm > 0.0 {
                // If legacy strongly disagrees, slightly down-weight confidence.
                // This helps downstream beat-tracking avoid over-trusting borderline tempos,
                // while preserving the tempogram BPM choice.
                conf = (conf * 0.90).clamp(0.0, 1.0);
                log::debug!(
                    "BPM fusion (validator): tempogram {:.2} kept; legacy {:.2} disagrees; conf {:.3}->{:.3}; temp_agree={}",
                    t_bpm,
                    l_bpm,
                    t_conf,
                    conf,
                    t_agree
                );
            } else {
                log::debug!(
                    "BPM fusion (validator): tempogram {:.2} kept; no legacy estimate available; temp_agree={}",
                    t_bpm,
                    t_agree
                );
            }

            (t_bpm, conf)
        }
    } else {
        // Default behavior: tempogram first; legacy fallback only if tempogram fails.
        tempogram_estimate
            .as_ref()
            .map(|e| (e.bpm, e.confidence))
            .or_else(|| legacy_estimate.as_ref().map(|e| (e.bpm, e.confidence)))
            .unwrap_or((0.0, 0.0))
    };
    
    if bpm == 0.0 && detect_bpm {
        log::warn!("Could not estimate BPM: tempogram and legacy methods both failed");
    } else {
        log::debug!("Estimated BPM: {:.2} (confidence: {:.3})", bpm, bpm_confidence);
    }
    
    // Phase 1C: Beat Tracking
    let (beat_grid, grid_stability) = if bpm > 0.0 && onsets_for_beat_tracking.len() >= 2 {
        // Convert onsets from sample indices to seconds
        let onsets_seconds: Vec<f32> = onsets_for_beat_tracking
            .iter()
            .map(|&sample_idx| sample_idx as f32 / sample_rate as f32)
            .collect();

        // Generate beat grid using HMM Viterbi algorithm
        use features::beat_tracking::generate_beat_grid;
        match generate_beat_grid(bpm, bpm_confidence, &onsets_seconds, sample_rate) {
            Ok((grid, stability)) => {
                log::debug!(
                    "Beat grid generated: {} beats, {} downbeats, stability={:.3}",
                    grid.beats.len(),
                    grid.downbeats.len(),
                    stability
                );
                (grid, stability)
            }
            Err(e) => {
                log::warn!("Beat tracking failed: {}, using empty grid", e);
                (
                    BeatGrid {
                        downbeats: vec![],
                        beats: vec![],
                        bars: vec![],
                    },
                    0.0,
                )
            }
        }
    } else {
        log::debug!("Skipping beat tracking: BPM={:.2}, onsets={}", bpm, energy_onsets.len());
        (
            BeatGrid {
                downbeats: vec![],
                beats: vec![],
                bars: vec![],
            },
            0.0,
        )
    };
    
    // Phase 1D: Key Detection
    let (key, key_confidence, key_clarity) = if detect_key && trimmed_samples.len() >= config.frame_size {
        // Extract chroma vectors with configurable options
        //
        // IMPORTANT: We already computed the STFT magnitudes for tempogram BPM. Reuse them here
        // to avoid a second STFT pass (and to enable spectrogram-domain conditioning for key).
        use features::chroma::extractor::{
            convert_linear_to_log_frequency_spectrogram,
            extract_beat_synchronous_chroma,
            extract_chroma_from_log_frequency_spectrogram,
            extract_chroma_from_spectrogram_with_options_and_energy,
            extract_chroma_from_spectrogram_with_options_and_energy_tuned,
            extract_hpcp_from_spectrogram_with_options_and_energy_tuned,
            extract_hpcp_bass_blend_from_spectrogram_with_options_and_energy_tuned,
            estimate_tuning_offset_semitones_from_spectrogram,
            harmonic_spectrogram_hpss_median_mask,
            harmonic_spectrogram_time_mask,
            smooth_spectrogram_time,
        };
        use features::chroma::normalization::sharpen_chroma;
        use features::chroma::smoothing::smooth_chroma;
        use features::key::{
            compute_key_clarity, detect_key_ensemble, detect_key_multi_scale, detect_key_weighted, detect_key_weighted_mode_heuristic,
            KeyDetectionResult, KeyTemplates,
        };
        
        // Key-only STFT override (optional): allow higher frequency resolution for key detection.
        let key_fft_size = if config.enable_key_stft_override {
            config.key_stft_frame_size.max(256)
        } else {
            config.frame_size
        };
        let key_hop_size = if config.enable_key_stft_override {
            config.key_stft_hop_size.max(1)
        } else {
            config.hop_size
        };

        let key_spec_frames = if config.enable_key_stft_override {
            match compute_stft(&trimmed_samples, key_fft_size, key_hop_size) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("Key-only STFT override failed: {}, falling back to shared STFT", e);
                    magnitude_spec_frames.clone()
                }
            }
        } else {
            magnitude_spec_frames.clone()
        };

        // Key-only spectrogram conditioning: suppress percussive broadband transients.
        let spec_for_key = if !key_spec_frames.is_empty() {
            if config.enable_key_hpss_harmonic {
                match harmonic_spectrogram_hpss_median_mask(
                    &key_spec_frames,
                    sample_rate,
                    key_fft_size,
                    100.0,
                    5000.0,
                    config.key_hpss_frame_step,
                    config.key_hpss_time_margin,
                    config.key_hpss_freq_margin,
                    config.key_hpss_mask_power,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Key HPSS harmonic mask failed: {}, falling back", e);
                        key_spec_frames.clone()
                    }
                }
            } else if config.enable_key_harmonic_mask {
                match harmonic_spectrogram_time_mask(
                    &key_spec_frames,
                    config.key_spectrogram_smooth_margin,
                    config.key_harmonic_mask_power,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Key harmonic mask failed: {}, falling back", e);
                        key_spec_frames.clone()
                    }
                }
            } else if config.enable_key_spectrogram_time_smoothing {
                match smooth_spectrogram_time(&key_spec_frames, config.key_spectrogram_smooth_margin) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Key spectrogram time smoothing failed: {}, using raw spectrogram", e);
                        key_spec_frames.clone()
                    }
                }
            } else {
                key_spec_frames.clone()
            }
        } else {
            key_spec_frames.clone()
        };

        // Optional: convert to log-frequency (semitone-aligned) spectrogram for key detection.
        // When enabled, HPCP is disabled (HPCP requires frequency information for harmonic summation).
        let (use_log_freq, log_freq_spec, semitone_offset) = if config.enable_key_log_frequency && !spec_for_key.is_empty() {
            match convert_linear_to_log_frequency_spectrogram(
                &spec_for_key,
                sample_rate,
                key_fft_size,
                100.0,
                5000.0,
            ) {
                Ok(log_spec) => {
                    // Compute semitone offset of first bin
                    let fmin: f32 = 100.0;
                    let semitone_min: f32 = 12.0 * (fmin / 440.0).log2() + 57.0;
                    let semitone_bin_min = semitone_min.floor() as i32;
                    (true, log_spec, semitone_bin_min)
                }
                Err(e) => {
                    log::warn!("Key log-frequency conversion failed: {}, falling back to linear", e);
                    (false, vec![], 0)
                }
            }
        } else {
            (false, vec![], 0)
        };

        // Optional: estimate tuning offset (in semitones) and apply during chroma mapping.
        // Note: tuning estimation still uses linear spectrogram (before log-freq conversion).
        let tuning_offset = if config.enable_key_tuning_compensation && !spec_for_key.is_empty() && !use_log_freq {
            match estimate_tuning_offset_semitones_from_spectrogram(
                &spec_for_key,
                sample_rate,
                key_fft_size,
                80.0,
                2000.0,
                config.key_tuning_frame_step,
                config.key_tuning_peak_rel_threshold,
            ) {
                Ok(d) => d.clamp(
                    -config.key_tuning_max_abs_semitones.abs(),
                    config.key_tuning_max_abs_semitones.abs(),
                ),
                Err(e) => {
                    log::warn!("Key tuning estimation failed: {}", e);
                    0.0
                }
            }
        } else {
            0.0
        };

        let chroma_call = if config.enable_key_beat_synchronous && !beat_grid.beats.is_empty() && !use_log_freq {
            // Beat-synchronous chroma: align chroma windows to beat boundaries
            extract_beat_synchronous_chroma(
                &spec_for_key,
                sample_rate,
                key_fft_size,
                key_hop_size,
                &beat_grid.beats,
                config.soft_chroma_mapping,
                config.soft_mapping_sigma,
                tuning_offset,
            )
        } else if use_log_freq {
            // Extract chroma from log-frequency spectrogram (each bin is already a semitone)
            extract_chroma_from_log_frequency_spectrogram(&log_freq_spec, semitone_offset)
                .map(|chroma_vecs| {
                    // Compute frame energies from log-frequency spectrogram
                    let energies: Vec<f32> = log_freq_spec
                        .iter()
                        .map(|frame| frame.iter().map(|&x| x * x).sum())
                        .collect();
                    (chroma_vecs, energies)
                })
        } else if config.enable_key_hpcp {
            if config.enable_key_hpcp_bass_blend {
                extract_hpcp_bass_blend_from_spectrogram_with_options_and_energy_tuned(
                    &spec_for_key,
                    sample_rate,
                    key_fft_size,
                    config.soft_mapping_sigma,
                    tuning_offset,
                    config.key_hpcp_peaks_per_frame,
                    config.key_hpcp_num_harmonics,
                    config.key_hpcp_harmonic_decay,
                    config.key_hpcp_mag_power,
                    config.enable_key_hpcp_whitening,
                    config.key_hpcp_whitening_smooth_bins,
                    config.key_hpcp_bass_fmin_hz,
                    config.key_hpcp_bass_fmax_hz,
                    config.key_hpcp_bass_weight,
                )
            } else {
                extract_hpcp_from_spectrogram_with_options_and_energy_tuned(
                    &spec_for_key,
                    sample_rate,
                    key_fft_size,
                    config.soft_mapping_sigma,
                    tuning_offset,
                    config.key_hpcp_peaks_per_frame,
                    config.key_hpcp_num_harmonics,
                    config.key_hpcp_harmonic_decay,
                    config.key_hpcp_mag_power,
                    config.enable_key_hpcp_whitening,
                    config.key_hpcp_whitening_smooth_bins,
                )
            }
        } else if config.enable_key_tuning_compensation && tuning_offset.abs() > 1e-6 {
            extract_chroma_from_spectrogram_with_options_and_energy_tuned(
                &spec_for_key,
                sample_rate,
                key_fft_size,
                config.soft_chroma_mapping,
                config.soft_mapping_sigma,
                tuning_offset,
            )
        } else {
            extract_chroma_from_spectrogram_with_options_and_energy(
                &spec_for_key,
                sample_rate,
                key_fft_size,
                config.soft_chroma_mapping,
                config.soft_mapping_sigma,
            )
        };

        match chroma_call {
            Ok((mut chroma_vectors, frame_energies)) => {
                // Apply chroma sharpening if enabled (power > 1.0)
                if config.chroma_sharpening_power > 1.0 {
                    for chroma in &mut chroma_vectors {
                        *chroma = sharpen_chroma(chroma, config.chroma_sharpening_power);
                    }
                    log::debug!("Applied chroma sharpening with power {:.2}", config.chroma_sharpening_power);
                }
                
                // Apply temporal smoothing (optional but recommended)
                if chroma_vectors.len() > 5 {
                    chroma_vectors = smooth_chroma(&chroma_vectors, 5);
                }

                // Optional edge trimming: remove intro/outro frames (often beat-only / percussive),
                // focusing key detection on the more harmonically informative middle section.
                let (chroma_slice, energy_slice): (&[Vec<f32>], &[f32]) = if config.enable_key_edge_trim
                    && chroma_vectors.len() == frame_energies.len()
                    && chroma_vectors.len() >= 200
                {
                    let frac = config.key_edge_trim_fraction.clamp(0.0, 0.49);
                    let n = chroma_vectors.len();
                    let start = ((n as f32) * frac).round() as usize;
                    let end = ((n as f32) * (1.0 - frac)).round() as usize;
                    if end > start + 50 && end <= n {
                        (&chroma_vectors[start..end], &frame_energies[start..end])
                    } else {
                        (&chroma_vectors[..], &frame_energies[..])
                    }
                } else {
                    (&chroma_vectors[..], &frame_energies[..])
                };

                // Build per-frame weights (optional): normalize energy by median, combine with tonalness.
                fn chroma_tonalness(chroma: &[f32]) -> f32 {
                    let sum: f32 = chroma.iter().sum();
                    if sum <= 1e-12 {
                        return 0.0;
                    }
                    let mut entropy = 0.0f32;
                    for &x in chroma {
                        let p = x / sum;
                        if p > 1e-12 {
                            entropy -= p * p.ln();
                        }
                    }
                    let max_entropy = (12.0f32).ln();
                    let t = 1.0 - (entropy / max_entropy);
                    t.clamp(0.0, 1.0)
                }
                
                let mut frame_weights: Option<Vec<f32>> = if config.enable_key_frame_weighting
                    && !energy_slice.is_empty()
                    && energy_slice.len() == chroma_slice.len()
                {
                    // Median energy for scale normalization.
                    let mut sorted = energy_slice.to_vec();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let median = sorted[sorted.len() / 2].max(1e-12);

                    let mut weights = Vec::with_capacity(chroma_slice.len());
                    for (ch, &e) in chroma_slice.iter().zip(energy_slice.iter()) {
                        let tonal = chroma_tonalness(ch);
                        let tonal = if tonal < config.key_min_tonalness { 0.0 } else { tonal };
                        let e_norm = (e / median).max(0.0);
                        let w_t = tonal.powf(config.key_tonalness_power.max(0.0));
                        let w_e = e_norm.powf(config.key_energy_power.max(0.0));
                        weights.push((w_t * w_e).max(0.0));
                    }
                    Some(weights)
                } else {
                    None
                };

                // Safety: if weighting zeroes out essentially everything, fall back to unweighted.
                if let Some(w) = frame_weights.as_ref() {
                    let sum_w: f32 = w.iter().sum();
                    let used = w.iter().filter(|&&x| x > 0.0).count();
                    if sum_w <= 1e-12 || used < 10 {
                        frame_weights = None;
                    }
                }

                // Optional: ensemble key detection (combine K-K and Temperley template scores).
                let key_call: Result<KeyDetectionResult, AnalysisError> = if config.enable_key_ensemble {
                    detect_key_ensemble(
                        chroma_slice,
                        frame_weights.as_deref(),
                        config.key_ensemble_kk_weight,
                        config.key_ensemble_temperley_weight,
                    )
                // Optional: multi-scale key detection (ensemble voting across multiple time scales).
                } else {
                    // Detect key using templates (selected template set)
                    let templates = KeyTemplates::new_with_template_set(config.key_template_set);
                    
                    if config.enable_key_multi_scale
                        && !config.key_multi_scale_lengths.is_empty()
                        && chroma_slice.len() >= *config.key_multi_scale_lengths.iter().min().unwrap_or(&1)
                    {
                        detect_key_multi_scale(
                            chroma_slice,
                            &templates,
                            frame_weights.as_deref(),
                            &config.key_multi_scale_lengths,
                            config.key_multi_scale_hop.max(1),
                            config.key_multi_scale_min_clarity.clamp(0.0, 1.0),
                            if config.key_multi_scale_weights.is_empty() {
                                None
                            } else {
                                Some(&config.key_multi_scale_weights)
                            },
                            config.enable_key_mode_heuristic,
                            config.key_mode_third_ratio_margin,
                            if config.enable_key_mode_heuristic {
                                config.key_mode_flip_min_score_ratio
                            } else {
                                0.0
                            },
                            config.enable_key_minor_harmonic_bonus,
                            config.key_minor_leading_tone_bonus_weight,
                        )
                    // Optional: segment voting (windowed detection + clarity-weighted score accumulation).
                    } else if config.enable_key_segment_voting
                    && chroma_slice.len() >= config.key_segment_len_frames.max(1)
                    && config.key_segment_len_frames >= 120
                    && config.key_segment_hop_frames >= 1
                {
                    let seg_len = config.key_segment_len_frames.min(chroma_slice.len());
                    let hop = config.key_segment_hop_frames.min(seg_len).max(1);
                    let min_clarity = config.key_segment_min_clarity.clamp(0.0, 1.0);

                    let mut acc_scores: Vec<(Key, f32)> = Vec::with_capacity(24);
                    // init score table
                    for k in 0..12 {
                        acc_scores.push((Key::Major(k as u32), 0.0));
                    }
                    for k in 0..12 {
                        acc_scores.push((Key::Minor(k as u32), 0.0));
                    }

                    let mut used_segments = 0usize;
                    let mut start = 0usize;
                    while start + seg_len <= chroma_slice.len() {
                        let seg = &chroma_slice[start..start + seg_len];
                        let wseg = frame_weights
                            .as_ref()
                            .map(|w| &w[start..start + seg_len]);
                        let seg_res = if config.enable_key_mode_heuristic || config.enable_key_minor_harmonic_bonus {
                            detect_key_weighted_mode_heuristic(
                                seg,
                                &templates,
                                wseg,
                                config.key_mode_third_ratio_margin,
                                if config.enable_key_mode_heuristic {
                                    config.key_mode_flip_min_score_ratio
                                } else {
                                    0.0
                                },
                                config.enable_key_minor_harmonic_bonus,
                                config.key_minor_leading_tone_bonus_weight,
                            )?
                        } else {
                            detect_key_weighted(seg, &templates, wseg)?
                        };
                        let seg_clarity = compute_key_clarity(&seg_res.all_scores);
                        if seg_clarity >= min_clarity {
                            used_segments += 1;
                            // Add all scores, weighted by clarity
                            for (k, s) in seg_res.all_scores.iter() {
                                if let Some((_kk, dst)) = acc_scores.iter_mut().find(|(kk, _)| kk == k) {
                                    *dst += *s * seg_clarity;
                                }
                            }
                        }
                        start += hop;
                    }

                    if used_segments == 0 {
                        if config.enable_key_mode_heuristic || config.enable_key_minor_harmonic_bonus {
                            detect_key_weighted_mode_heuristic(
                                chroma_slice,
                                &templates,
                                frame_weights.as_deref(),
                                config.key_mode_third_ratio_margin,
                                if config.enable_key_mode_heuristic {
                                    config.key_mode_flip_min_score_ratio
                                } else {
                                    0.0
                                },
                                config.enable_key_minor_harmonic_bonus,
                                config.key_minor_leading_tone_bonus_weight,
                            )
                        } else {
                            detect_key_weighted(chroma_slice, &templates, frame_weights.as_deref())
                        }
                    } else {
                        // Sort accumulated scores and build a KeyDetectionResult.
                        acc_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                        let (best_key, best_score) = acc_scores[0];
                        let second_score = if acc_scores.len() > 1 { acc_scores[1].1 } else { 0.0 };
                        let confidence = if best_score > 0.0 {
                            ((best_score - second_score) / best_score).max(0.0).min(1.0)
                        } else {
                            0.0
                        };
                        let top_n = 3usize.min(acc_scores.len());
                        let top_keys = acc_scores.iter().take(top_n).cloned().collect::<Vec<_>>();
                        Ok(KeyDetectionResult {
                            key: best_key,
                            confidence,
                            all_scores: acc_scores,
                            top_keys,
                        })
                    }
                } else if config.enable_key_mode_heuristic || config.enable_key_minor_harmonic_bonus {
                    detect_key_weighted_mode_heuristic(
                        chroma_slice,
                        &templates,
                        frame_weights.as_deref(),
                        config.key_mode_third_ratio_margin,
                        if config.enable_key_mode_heuristic {
                            config.key_mode_flip_min_score_ratio
                        } else {
                            0.0
                        },
                        config.enable_key_minor_harmonic_bonus,
                        config.key_minor_leading_tone_bonus_weight,
                    )
                } else {
                    detect_key_weighted(chroma_slice, &templates, frame_weights.as_deref())
                }
            };

                match key_call {
                    Ok(key_result) => {
                        // Compute key clarity
                        let clarity = compute_key_clarity(&key_result.all_scores);
                        
                        log::debug!("Detected key: {:?}, confidence: {:.3}, clarity: {:.3}",
                                   key_result.key, key_result.confidence, clarity);

                        // Optional debug dump to stderr (captured by validation harness)
                        if let Some(track_id) = config.debug_track_id {
                            // Weighted pitch-class summary (for diagnosing collapses)
                            let mut agg = vec![0.0f32; 12];
                            let mut used = 0usize;
                            for (idx, ch) in chroma_slice.iter().enumerate() {
                                let w = frame_weights
                                    .as_ref()
                                    .and_then(|v| v.get(idx).copied())
                                    .unwrap_or(1.0);
                                if w <= 0.0 {
                                    continue;
                                }
                                used += 1;
                                for i in 0..12 {
                                    agg[i] += w * ch[i];
                                }
                            }
                            let sum_agg: f32 = agg.iter().sum();
                            if sum_agg > 1e-12 {
                                for x in agg.iter_mut() {
                                    *x /= sum_agg;
                                }
                            }
                            let mut pcs: Vec<(usize, f32)> = agg.iter().cloned().enumerate().collect();
                            pcs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                            eprintln!("\n=== DEBUG key (track_id={}) ===", track_id);
                            eprintln!(
                                "key={} conf={:.4} clarity={:.4} frames={} used_frames={} soft_mapping={} sigma={:.3} harmonic_mask={} mask_p={:.2} tuning={:.4} time_smooth={} margin={} edge_trim={} trim_frac={:.2}",
                                key_result.key.name(),
                                key_result.confidence,
                                clarity,
                                chroma_slice.len(),
                                used,
                                config.soft_chroma_mapping,
                                config.soft_mapping_sigma,
                                config.enable_key_harmonic_mask,
                                config.key_harmonic_mask_power,
                                tuning_offset,
                                config.enable_key_spectrogram_time_smoothing,
                                config.key_spectrogram_smooth_margin,
                                config.enable_key_edge_trim,
                                config.key_edge_trim_fraction
                            );
                            eprintln!(
                                "top_keys: {}",
                                key_result
                                    .top_keys
                                    .iter()
                                    .map(|(k, s)| format!("{}:{:.4}", k.name(), s))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            let note_names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
                            eprintln!(
                                "top_pitch_classes(weighted): {}",
                                pcs.iter()
                                    .take(6)
                                    .map(|(i, v)| format!("{}:{:.3}", note_names[*i], v))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                        
                        (key_result.key, key_result.confidence, clarity)
                    }
                    Err(e) => {
                        log::warn!("Key detection failed: {}, using default", e);
                        (Key::Major(0), 0.0, 0.0)
                    }
                }
            }
            Err(e) => {
                log::warn!("Chroma extraction failed: {}, using default key", e);
                (Key::Major(0), 0.0, 0.0)
            }
        }
    } else if !detect_key {
        log::debug!("Skipping key detection (BPM only)");
        (Key::Major(0), 0.0, 0.0)
    } else {
        log::debug!("Skipping key detection: insufficient samples (need at least {} samples)",
                   config.frame_size);
        (Key::Major(0), 0.0, 0.0)
    };
    
    let processing_time_ms = start_time.elapsed().as_secs_f32() * 1000.0;
    
    // Build confidence warnings
    let mut confidence_warnings = Vec::new();
    let mut flags = Vec::new();
    
    if bpm == 0.0 && detect_bpm {
        confidence_warnings.push("BPM detection failed: insufficient onsets or estimation error".to_string());
    }
    if grid_stability < 0.5 && detect_bpm {
        confidence_warnings.push(format!("Low beat grid stability: {:.2} (may indicate tempo variation)", grid_stability));
    }
    if key_confidence < 0.3 && detect_key {
        confidence_warnings.push(format!("Low key detection confidence: {:.2} (may indicate ambiguous or atonal music)", key_confidence));
    }
    if key_clarity < 0.2 && detect_key {
        confidence_warnings.push(format!("Low key clarity: {:.2} (track may be atonal or have weak tonality)", key_clarity));
        flags.push(crate::analysis::result::AnalysisFlag::WeakTonality);
    }
    
    // Phase 1E: Build result and compute comprehensive confidence scores
    let result = AnalysisResult {
        bpm,
        bpm_confidence,
        key,
        key_confidence,
        key_clarity,
        beat_grid,
        grid_stability,
        metadata: AnalysisMetadata {
            duration_seconds: trimmed_samples.len() as f32 / sample_rate as f32,
            sample_rate,
            processing_time_ms,
            algorithm_version: "0.1.0-alpha".to_string(),
            onset_method_consensus: if energy_onsets.is_empty() { 0.0 } else { 1.0 },
            methods_used: vec!["energy_flux".to_string(), "chroma_extraction".to_string(), "key_detection".to_string()],
            flags,
            confidence_warnings,
            tempogram_candidates,
            tempogram_multi_res_triggered,
            tempogram_multi_res_used,
            tempogram_percussive_triggered,
            tempogram_percussive_used,
        },
    };
    
    // Phase 1E: Compute comprehensive confidence scores
    use analysis::confidence::compute_confidence;
    let confidence = compute_confidence(&result);
    log::debug!(
        "Analysis complete: BPM={:.2} (conf={:.3}), Key={:?} (conf={:.3}), Overall confidence={:.3}",
        result.bpm,
        confidence.bpm_confidence,
        result.key,
        confidence.key_confidence,
        confidence.overall_confidence
    );
    
    // Return result with Phase 1E confidence scoring integrated
    Ok(result)
}

