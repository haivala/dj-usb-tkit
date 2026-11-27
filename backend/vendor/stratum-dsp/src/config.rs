//! Configuration parameters for audio analysis

use crate::preprocessing::normalization::NormalizationMethod;
use crate::features::key::templates::TemplateSet;

/// Analysis configuration parameters
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    // Preprocessing
    /// Silence detection threshold in dB (default: -40.0)
    /// Frames with RMS below this threshold are considered silent
    pub min_amplitude_db: f32,
    
    /// Normalization method to use (default: Peak)
    pub normalization: NormalizationMethod,

    /// Enable normalization step (default: true)
    pub enable_normalization: bool,

    /// Enable silence detection + trimming step (default: true)
    pub enable_silence_trimming: bool,

    // Onset detection (used by beat tracking and legacy BPM fallback)
    /// Enable multi-detector onset consensus (spectral flux + HFC + optional HPSS) (default: true)
    ///
    /// Note: Tempogram BPM does not use this onset list, but legacy BPM + beat tracking do.
    pub enable_onset_consensus: bool,

    /// Threshold percentile for STFT-based onset detectors (spectral flux / HFC / HPSS) (default: 0.80)
    /// Range: [0.0, 1.0]
    pub onset_threshold_percentile: f32,

    /// Onset clustering tolerance window in milliseconds for consensus voting (default: 50 ms)
    pub onset_consensus_tolerance_ms: u32,

    /// Consensus method weights [energy_flux, spectral_flux, hfc, hpss] (default: equal weights)
    pub onset_consensus_weights: [f32; 4],

    /// Enable HPSS-based onset detector inside consensus (default: false; more expensive)
    pub enable_hpss_onsets: bool,

    /// HPSS median-filter margin (default: 10). Typical values: 5–20.
    pub hpss_margin: usize,
    
    // BPM detection
    /// Force legacy BPM estimation (Phase 1B autocorrelation + comb filter) and skip tempogram.
    /// Default: false.
    ///
    /// Intended for A/B validation and hybrid/consensus experimentation.
    pub force_legacy_bpm: bool,

    /// Enable BPM fusion (compute tempogram + legacy in parallel, then choose using consensus logic).
    /// Default: false (tempogram-only unless it fails, then legacy fallback).
    pub enable_bpm_fusion: bool,

    /// Enable legacy BPM guardrails (soft confidence caps by tempo range).
    /// Default: true.
    pub enable_legacy_bpm_guardrails: bool,

    /// Enable **true** multi-resolution tempogram BPM estimation.
    ///
    /// When enabled, BPM estimation recomputes STFT at hop sizes {256, 512, 1024} and fuses
    /// candidates using a cross-resolution scoring rule. This is intended to reduce
    /// metrical-level (T vs 2T vs T/2) errors.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_multi_resolution: bool,

    /// Multi-resolution fusion: number of hop=512 candidates to consider as anchors.
    /// Default: 10.
    pub tempogram_multi_res_top_k: usize,

    /// Multi-resolution fusion weight for hop=512 (global beat).
    pub tempogram_multi_res_w512: f32,
    /// Multi-resolution fusion weight for hop=256 (fine transients).
    pub tempogram_multi_res_w256: f32,
    /// Multi-resolution fusion weight for hop=1024 (structural/metre level).
    pub tempogram_multi_res_w1024: f32,

    /// Structural discount factor applied when hop=1024 supports 2T instead of T.
    pub tempogram_multi_res_structural_discount: f32,

    /// Factor applied to hop=512 support when evaluating the 2T / T/2 hypotheses.
    pub tempogram_multi_res_double_time_512_factor: f32,

    /// Minimum score margin (absolute) required to switch between T / 2T / T/2 hypotheses.
    pub tempogram_multi_res_margin_threshold: f32,

    /// Enable a gentle human-tempo prior as a tie-breaker (only when scores are very close).
    /// Default: false.
    pub tempogram_multi_res_use_human_prior: bool,

    /// Enable HPSS percussive-only tempogram fallback (ambiguous-only).
    ///
    /// This computes an HPSS decomposition on the (already computed) STFT magnitudes and re-runs
    /// tempogram on the percussive component. Intended to reduce low-tempo half/double-time traps
    /// caused by sustained harmonic energy.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_percussive_fallback: bool,

    /// Enable multi-band novelty fusion inside the tempogram estimator.
    ///
    /// This computes novelty curves over low/mid/high frequency bands, runs the tempogram
    /// on each, then fuses their support when scoring BPM candidates. This is primarily
    /// intended to improve **candidate generation** (getting GT into top-N candidates),
    /// which is currently the limiting factor after metrical selection improvements.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_band_fusion: bool,

    /// Band split cutoffs (Hz). Bands are: low=[~0..low_max], mid=[low_max..mid_max], high=[mid_max..high_max].
    /// If `tempogram_band_high_max_hz <= 0`, high extends to Nyquist.
    pub tempogram_band_low_max_hz: f32,
    /// Upper cutoff for the mid band (Hz).
    pub tempogram_band_mid_max_hz: f32,
    /// Upper cutoff for the high band (Hz). If <= 0, uses Nyquist.
    pub tempogram_band_high_max_hz: f32,

    /// Weight for the full-band tempogram contribution when band-score fusion is enabled.
    pub tempogram_band_w_full: f32,
    /// Weight for the low band contribution.
    pub tempogram_band_w_low: f32,
    /// Weight for the mid band contribution.
    pub tempogram_band_w_mid: f32,
    /// Weight for the high band contribution.
    pub tempogram_band_w_high: f32,

    /// If true, multi-band tempograms contribute **only to candidate seeding** (peak proposals),
    /// while final candidate scoring remains full-band-only.
    ///
    /// This is the safer default: high-frequency bands often emphasize subdivisions (hi-hats),
    /// which can otherwise increase 2× / 3:2 metrical errors if they directly affect scoring.
    pub tempogram_band_seed_only: bool,

    /// Minimum per-band normalized support required to count as "supporting" a BPM candidate
    /// for band-consensus scoring.
    ///
    /// Range: [0, 1]. Default: 0.25.
    pub tempogram_band_support_threshold: f32,

    /// Bonus multiplier applied when **multiple bands** support the same BPM candidate.
    ///
    /// This is a lightweight "consensus" heuristic intended to reduce metrical/subdivision errors
    /// (e.g., a 2× tempo supported only by the high band should not win over a tempo supported by
    /// low+mid bands).
    ///
    /// Score adjustment: `score *= (1 + bonus * max(0, support_bands - 1))`.
    pub tempogram_band_consensus_bonus: f32,

    /// Tempogram novelty weights for combining {spectral, energy, HFC}.
    pub tempogram_novelty_w_spectral: f32,
    /// Tempogram novelty weight for energy flux.
    pub tempogram_novelty_w_energy: f32,
    /// Tempogram novelty weight for HFC.
    pub tempogram_novelty_w_hfc: f32,
    /// Tempogram novelty conditioning windows.
    pub tempogram_novelty_local_mean_window: usize,
    /// Tempogram novelty moving-average smoothing window (frames). Use 0/1 to disable.
    pub tempogram_novelty_smooth_window: usize,

    /// Debug: if set, the `analyze_file` example will pass this track ID through to the
    /// multi-resolution fusion so it can print detailed scoring diagnostics.
    pub debug_track_id: Option<u32>,
    /// Debug: optional ground-truth BPM passed alongside `debug_track_id`.
    pub debug_gt_bpm: Option<f32>,
    /// Debug: number of top candidates per hop to print when `debug_track_id` is set.
    pub debug_top_n: usize,

    /// Enable log-mel novelty tempogram as an additional candidate generator/support signal.
    ///
    /// This computes a log-mel SuperFlux-style novelty curve, then runs the tempogram on it.
    /// The resulting candidates are used for seeding and for the consensus bonus logic.
    pub enable_tempogram_mel_novelty: bool,
    /// Mel band count used by log-mel novelty.
    pub tempogram_mel_n_mels: usize,
    /// Minimum mel frequency (Hz).
    pub tempogram_mel_fmin_hz: f32,
    /// Maximum mel frequency (Hz). If <= 0, uses Nyquist.
    pub tempogram_mel_fmax_hz: f32,
    /// Max-filter neighborhood radius in mel bins (SuperFlux-style reference).
    pub tempogram_mel_max_filter_bins: usize,
    /// Weight for mel variant when band scoring fusion is enabled (`seed_only=false`).
    pub tempogram_mel_weight: f32,

    /// SuperFlux max-filter neighborhood radius (bins) used by the tempogram novelty extractor.
    pub tempogram_superflux_max_filter_bins: usize,

    /// Emit tempogram BPM candidate list (top-N) into `AnalysisMetadata` for validation/tuning.
    ///
    /// Default: false (avoid bloating outputs in normal use).
    pub emit_tempogram_candidates: bool,

    /// Number of tempogram candidates to emit when `emit_tempogram_candidates` is enabled.
    /// Default: 10.
    pub tempogram_candidates_top_n: usize,

    /// Legacy guardrails: preferred BPM range (default: 75–150).
    pub legacy_bpm_preferred_min: f32,
    /// Legacy guardrails: preferred BPM range upper bound (default: 150).
    pub legacy_bpm_preferred_max: f32,

    /// Legacy guardrails: soft BPM range (default: 60–180).
    /// Values in [soft_min, preferred_min) or (preferred_max, soft_max] get a medium cap.
    pub legacy_bpm_soft_min: f32,
    /// Legacy guardrails: soft BPM range upper bound (default: 180).
    pub legacy_bpm_soft_max: f32,

    /// Legacy guardrails: confidence caps by range.
    /// - preferred: inside [preferred_min, preferred_max]
    /// - soft: inside [soft_min, soft_max] but outside preferred
    /// - extreme: outside [soft_min, soft_max]
    ///
    /// **Multiplier semantics**: these are applied as `confidence *= multiplier` to legacy
    /// candidates/estimates (softly biasing the selection).
    pub legacy_bpm_conf_mul_preferred: f32,
    /// Legacy guardrails: confidence multiplier for the soft band (default: 0.50).
    pub legacy_bpm_conf_mul_soft: f32,
    /// Legacy guardrails: confidence multiplier for extremes (default: 0.10).
    pub legacy_bpm_conf_mul_extreme: f32,

    /// Minimum BPM to consider (default: 60.0)
    pub min_bpm: f32,
    
    /// Maximum BPM to consider (default: 180.0)
    pub max_bpm: f32,
    
    /// BPM resolution for comb filterbank (default: 1.0)
    pub bpm_resolution: f32,
    
    // STFT parameters
    /// Frame size for STFT (default: 2048)
    pub frame_size: usize,
    
    /// Hop size for STFT (default: 512)
    pub hop_size: usize,
    
    // Key detection
    /// Center frequency for chroma extraction (default: 440.0 Hz, A4)
    pub center_frequency: f32,
    
    /// Enable soft chroma mapping (default: true)
    /// Soft mapping spreads frequency bins to neighboring semitones for robustness
    pub soft_chroma_mapping: bool,
    
    /// Soft mapping standard deviation in semitones (default: 0.5)
    /// Lower values = sharper mapping, higher values = more spread
    pub soft_mapping_sigma: f32,
    
    /// Chroma sharpening power (default: 1.0 = no sharpening, 1.5-2.0 recommended)
    /// Power > 1.0 emphasizes prominent semitones, improving key detection
    pub chroma_sharpening_power: f32,

    /// Enable a lightweight percussive-suppression step for key detection by time-smoothing
    /// the STFT magnitude spectrogram prior to chroma extraction.
    ///
    /// This is HPSS-inspired (harmonic content is sustained in time; percussive is transient),
    /// but uses a cheap moving-average rather than full iterative HPSS.
    ///
    /// Default: true.
    pub enable_key_spectrogram_time_smoothing: bool,

    /// Half-window size (in frames) for the key spectrogram time-smoothing.
    /// Effective window length is `2*margin + 1`.
    ///
    /// Default: 12 (≈ 12 * hop_size samples ≈ 140 ms at 44.1kHz with hop=512).
    pub key_spectrogram_smooth_margin: usize,

    /// Enable weighted key aggregation (frame weights based on tonality + energy).
    /// Default: true.
    pub enable_key_frame_weighting: bool,

    /// Minimum per-frame "tonalness" required to include the frame in key aggregation.
    /// Tonalness is computed from chroma entropy and mapped to [0, 1].
    /// Default: 0.10.
    pub key_min_tonalness: f32,

    /// Exponent applied to tonalness when building frame weights (>= 0).
    /// Default: 2.0.
    pub key_tonalness_power: f32,

    /// Exponent applied to normalized frame energy when building frame weights (>= 0).
    /// Default: 0.50 (square-root weighting).
    pub key_energy_power: f32,

    /// Enable a harmonic-emphasized spectrogram for key detection via a time-smoothing-derived
    /// soft mask (cheap HPSS-inspired).
    ///
    /// If enabled, key detection uses `harmonic_spectrogram_time_mask()` instead of raw/time-smoothed
    /// magnitudes when extracting chroma.
    ///
    /// Default: true.
    pub enable_key_harmonic_mask: bool,

    /// Soft-mask exponent \(p\) for harmonic masking (>= 1.0). Higher values produce harder masks.
    /// Default: 2.0.
    pub key_harmonic_mask_power: f32,

    /// Enable median-filter HPSS harmonic extraction for key detection (key-only).
    ///
    /// This is a more literature-standard HPSS step than `harmonic_spectrogram_time_mask()`.
    /// We compute time- and frequency-median estimates on a **time-downsampled**, **band-limited**
    /// spectrogram, build a soft mask, then apply it to the full-resolution spectrogram.
    ///
    /// Default: false (opt-in; more expensive).
    pub enable_key_hpss_harmonic: bool,

    /// Time-downsampling step for key HPSS (>= 1). Values like 2–6 greatly reduce cost.
    /// Default: 4.
    pub key_hpss_frame_step: usize,

    /// Half-window size (in downsampled frames) for the HPSS harmonic (time) median filter.
    /// Effective window length is `2*margin + 1` (in downsampled frames).
    /// Default: 8.
    pub key_hpss_time_margin: usize,

    /// Half-window size (in frequency bins) for the HPSS percussive (frequency) median filter.
    /// Effective window length is `2*margin + 1` bins.
    /// Default: 8.
    pub key_hpss_freq_margin: usize,

    /// Soft-mask exponent \(p\) for HPSS masking (>= 1.0). Higher values produce harder masks.
    /// Default: 2.0.
    pub key_hpss_mask_power: f32,

    /// Enable a key-only STFT override (compute a separate STFT for key detection).
    ///
    /// Rationale: key detection benefits from higher frequency resolution than BPM/onset work.
    /// A larger FFT size improves pitch precision at low frequencies where semitone spacing is small.
    ///
    /// Default: false (keep single shared STFT by default).
    pub enable_key_stft_override: bool,

    /// FFT frame size used for key-only STFT when `enable_key_stft_override` is true.
    /// Default: 8192.
    pub key_stft_frame_size: usize,

    /// Hop size used for key-only STFT when `enable_key_stft_override` is true.
    /// Default: 512.
    pub key_stft_hop_size: usize,

    /// Enable log-frequency (semitone-aligned) spectrogram for key detection.
    ///
    /// This converts the linear STFT magnitude spectrogram into a log-frequency representation
    /// where each bin corresponds to one semitone. This provides better pitch-class resolution
    /// than mapping linear FFT bins to semitones, especially at low frequencies.
    ///
    /// When enabled, chroma extraction works directly on semitone bins (no frequency-to-semitone
    /// mapping needed). HPCP is disabled when log-frequency is enabled (HPCP requires frequency
    /// information for harmonic summation).
    ///
    /// Default: false (use linear STFT with frequency-to-semitone mapping).
    pub enable_key_log_frequency: bool,

    /// Enable beat-synchronous chroma extraction for key detection.
    ///
    /// This aligns chroma windows to beat boundaries instead of fixed-time frames, improving
    /// harmonic coherence by aligning to musical structure. For each beat interval, chroma vectors
    /// from all STFT frames within that interval are averaged.
    ///
    /// Requires a valid beat grid (falls back to frame-based chroma if beat grid is unavailable).
    /// HPCP is disabled when beat-synchronous is enabled (HPCP requires frame-based processing).
    ///
    /// Default: false (use frame-based chroma extraction).
    pub enable_key_beat_synchronous: bool,

    /// Enable multi-scale key detection (ensemble voting across multiple time scales).
    ///
    /// This runs key detection at multiple segment lengths (short, medium, long) and aggregates
    /// results using clarity-weighted voting. This captures both local and global key information,
    /// improving robustness on tracks with key changes or varying harmonic stability.
    ///
    /// Default: false (use single-scale detection).
    pub enable_key_multi_scale: bool,

    /// Template set to use for key detection.
    ///
    /// - `KrumhanslKessler`: Krumhansl-Kessler (1982) templates (empirical, from listening experiments)
    /// - `Temperley`: Temperley (1999) templates (statistical, from corpus analysis)
    ///
    /// Default: `KrumhanslKessler`.
    pub key_template_set: crate::features::key::templates::TemplateSet,

    /// Enable ensemble key detection (combine K-K and Temperley template scores).
    ///
    /// This runs key detection with both template sets and combines their scores using
    /// weighted voting. This ensemble approach can improve robustness by leveraging
    /// complementary strengths of different template sets.
    ///
    /// Default: false (use single template set).
    pub enable_key_ensemble: bool,

    /// Weight for Krumhansl-Kessler scores in ensemble detection.
    ///
    /// Default: 0.5 (equal weight with Temperley).
    pub key_ensemble_kk_weight: f32,

    /// Weight for Temperley scores in ensemble detection.
    ///
    /// Default: 0.5 (equal weight with K-K).
    pub key_ensemble_temperley_weight: f32,

    /// Enable median key detection (detect key from multiple short segments and select median).
    ///
    /// This divides the track into multiple short overlapping segments, detects key for each
    /// segment, and selects the median key (most common key across segments). This helps
    /// handle brief modulations, breakdowns, or ambiguous sections.
    ///
    /// Default: false (use global key detection).
    pub enable_key_median: bool,

    /// Segment length (in frames) for median key detection.
    ///
    /// Default: 480 (~4 seconds at typical frame rates).
    pub key_median_segment_length_frames: usize,

    /// Segment hop size (in frames) for median key detection.
    ///
    /// Default: 120 (~1 second).
    pub key_median_segment_hop_frames: usize,

    /// Minimum number of segments required for median key detection.
    ///
    /// If fewer segments are available, falls back to global detection.
    ///
    /// Default: 3.
    pub key_median_min_segments: usize,

    /// Segment lengths (in frames) for multi-scale key detection.
    /// Multiple scales are processed and aggregated with clarity-weighted voting.
    /// Default: [120, 360, 720] (approximately 2s, 6s, 12s at typical frame rates).
    pub key_multi_scale_lengths: Vec<usize>,

    /// Hop size (in frames) between segments for multi-scale detection.
    /// Default: 60 (approximately 1s at typical frame rates).
    pub key_multi_scale_hop: usize,

    /// Minimum clarity threshold for including a segment in multi-scale aggregation.
    /// Default: 0.20.
    pub key_multi_scale_min_clarity: f32,

    /// Optional weights for each scale in multi-scale detection (if empty, all scales weighted equally).
    /// Length should match `key_multi_scale_lengths`. Default: empty (equal weights).
    pub key_multi_scale_weights: Vec<f32>,

    /// Enable per-track tuning compensation for key detection.
    ///
    /// This estimates a global detuning offset (in semitones, relative to A4=440Hz) from the
    /// key spectrogram, then shifts semitone mapping by that offset during chroma extraction.
    ///
    /// Default: true.
    pub enable_key_tuning_compensation: bool,

    /// Maximum absolute tuning correction to apply (semitones).
    /// Default: 0.25.
    pub key_tuning_max_abs_semitones: f32,

    /// Frame subsampling step used for tuning estimation (>= 1).
    /// Default: 20.
    pub key_tuning_frame_step: usize,

    /// Relative threshold (fraction of per-frame peak) for selecting bins used in tuning estimation.
    /// Default: 0.35.
    pub key_tuning_peak_rel_threshold: f32,

    /// Enable trimming the first/last fraction of frames for key detection.
    ///
    /// DJ tracks often have long beat-only intros/outros; trimming edges reduces percussive bias
    /// without affecting tempo (tempo uses its own pipeline).
    ///
    /// Default: true.
    pub enable_key_edge_trim: bool,

    /// Fraction (0..0.49) to trim from the start and end (symmetric) when `enable_key_edge_trim` is true.
    /// Default: 0.15 (use middle 70%).
    pub key_edge_trim_fraction: f32,

    /// Enable segment voting for key detection (windowed key detection + score accumulation).
    ///
    /// Rationale: long-form DJ tracks can modulate, have breakdowns, or contain beat-only sections.
    /// Segment voting helps focus on harmonically stable portions without requiring full key-change tracking.
    ///
    /// Default: true.
    pub enable_key_segment_voting: bool,

    /// Segment length in chroma frames for key voting.
    /// Default: 1024 (~11.9s at 44.1kHz, hop=512).
    pub key_segment_len_frames: usize,

    /// Segment hop/stride in frames for key voting.
    /// Default: 512 (~50% overlap).
    pub key_segment_hop_frames: usize,

    /// Minimum clarity required to include a segment in voting (0..1).
    /// Default: 0.20.
    pub key_segment_min_clarity: f32,

    /// Enable a conservative mode heuristic to reduce minor→major mistakes.
    ///
    /// Uses the 3rd degree (minor third vs major third) from the aggregated chroma to potentially
    /// flip parallel mode, gated by a score-ratio threshold.
    ///
    /// Default: true.
    pub enable_key_mode_heuristic: bool,

    /// Required ratio margin for the 3rd-degree test (>=0). If `p(min3) > p(maj3) * (1+margin)`
    /// we prefer minor (and vice versa for major).
    /// Default: 0.05.
    pub key_mode_third_ratio_margin: f32,

    /// Only flip parallel mode if the alternate mode's template score is at least this ratio of
    /// the best mode's score (0..1).
    /// Default: 0.92.
    pub key_mode_flip_min_score_ratio: f32,

    /// Enable HPCP-style pitch-class profile extraction for key detection.
    ///
    /// This uses spectral peak picking + harmonic summation to form a more robust tonal profile
    /// than raw STFT-bin chroma on real-world mixes.
    ///
    /// Default: false (experimental).
    pub enable_key_hpcp: bool,

    /// Number of spectral peaks per frame used for HPCP extraction.
    /// Default: 24.
    pub key_hpcp_peaks_per_frame: usize,

    /// Number of harmonics per peak used for HPCP extraction.
    /// Default: 4.
    pub key_hpcp_num_harmonics: usize,

    /// Harmonic decay factor applied per harmonic (0..1). Lower values emphasize fundamentals.
    /// Default: 0.60.
    pub key_hpcp_harmonic_decay: f32,

    /// Magnitude compression exponent for peak weights (0..1].
    /// Default: 0.50 (sqrt).
    pub key_hpcp_mag_power: f32,

    /// Enable spectral whitening (per-frame frequency-domain normalization) for HPCP peak picking.
    ///
    /// This suppresses timbral formants and broadband coloration, helping peaks corresponding to
    /// harmonic partials stand out more consistently across mixes.
    ///
    /// Default: false.
    pub enable_key_hpcp_whitening: bool,

    /// Frequency smoothing window (in FFT bins) for HPCP whitening.
    /// Larger values whiten more aggressively (more timbre suppression), but can also amplify noise.
    ///
    /// Default: 31.
    pub key_hpcp_whitening_smooth_bins: usize,

    /// Enable a bass-band HPCP blend (tonic reinforcement).
    ///
    /// Relative major/minor share pitch classes; bass/tonic emphasis can disambiguate mode in
    /// dance music where the bassline strongly implies the tonic.
    ///
    /// Default: true.
    pub enable_key_hpcp_bass_blend: bool,

    /// Bass-band lower cutoff (Hz) for bass HPCP.
    /// Default: 55.0.
    pub key_hpcp_bass_fmin_hz: f32,

    /// Bass-band upper cutoff (Hz) for bass HPCP.
    /// Default: 300.0.
    pub key_hpcp_bass_fmax_hz: f32,

    /// Blend weight for bass HPCP (0..1). Final PCP = normalize((1-w)*full + w*bass).
    /// Default: 0.35.
    pub key_hpcp_bass_weight: f32,

    /// Enable a minor-key harmonic bonus (leading-tone vs flat-7) when scoring templates.
    ///
    /// Many dance tracks in minor heavily use harmonic minor gestures (raised 7th). This bonus
    /// nudges minor candidates whose pitch-class distribution supports a leading-tone.
    ///
    /// Default: true.
    pub enable_key_minor_harmonic_bonus: bool,

    /// Weight for the minor harmonic bonus. Internally scaled by the sum of frame weights so it
    /// is comparable to the template-score scale.
    ///
    /// Default: 0.8.
    pub key_minor_leading_tone_bonus_weight: f32,
    
    // ML refinement
    /// Enable ML refinement (requires ml feature)
    #[cfg(feature = "ml")]
    pub enable_ml_refinement: bool,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            min_amplitude_db: -40.0,
            normalization: NormalizationMethod::Peak,
            enable_normalization: true,
            enable_silence_trimming: true,
            enable_onset_consensus: true,
            onset_threshold_percentile: 0.80,
            onset_consensus_tolerance_ms: 50,
            onset_consensus_weights: [0.25, 0.25, 0.25, 0.25],
            enable_hpss_onsets: false,
            hpss_margin: 10,
            force_legacy_bpm: false,
            enable_bpm_fusion: false,
            enable_legacy_bpm_guardrails: true,
            enable_tempogram_multi_resolution: true,
            tempogram_multi_res_top_k: 25,
            tempogram_multi_res_w512: 0.45,
            tempogram_multi_res_w256: 0.35,
            tempogram_multi_res_w1024: 0.20,
            tempogram_multi_res_structural_discount: 0.85,
            tempogram_multi_res_double_time_512_factor: 0.92,
            tempogram_multi_res_margin_threshold: 0.08,
            tempogram_multi_res_use_human_prior: false,
            // HPSS percussive fallback is very expensive and (so far) has not shown consistent gains.
            // Keep it opt-in to avoid multi-second outliers during batch runs.
            enable_tempogram_percussive_fallback: false,
            enable_tempogram_band_fusion: true,
            // Default cutoffs (Hz): ~kick/bass fundamentals, then body/rhythm textures, then attacks.
            tempogram_band_low_max_hz: 200.0,
            tempogram_band_mid_max_hz: 2000.0,
            tempogram_band_high_max_hz: 8000.0,
            // Default weights: keep full-band as anchor, but allow bands to pull candidates into view.
            tempogram_band_w_full: 0.40,
            tempogram_band_w_low: 0.25,
            tempogram_band_w_mid: 0.20,
            tempogram_band_w_high: 0.15,
            tempogram_band_seed_only: true,
            tempogram_band_support_threshold: 0.25,
            tempogram_band_consensus_bonus: 0.08,
            // Novelty weighting defaults (tuned on 200-track validation):
            // shift weight toward transient-heavy signals (energy/HFC) to reduce octave/subdivision traps.
            tempogram_novelty_w_spectral: 0.30,
            tempogram_novelty_w_energy: 0.35,
            tempogram_novelty_w_hfc: 0.35,
            tempogram_novelty_local_mean_window: 16,
            tempogram_novelty_smooth_window: 5,
            debug_track_id: None,
            debug_gt_bpm: None,
            debug_top_n: 5,
            enable_tempogram_mel_novelty: true,
            tempogram_mel_n_mels: 40,
            tempogram_mel_fmin_hz: 30.0,
            tempogram_mel_fmax_hz: 8000.0,
            tempogram_mel_max_filter_bins: 2,
            tempogram_mel_weight: 0.15,
            tempogram_superflux_max_filter_bins: 4,
            emit_tempogram_candidates: false,
            tempogram_candidates_top_n: 10,
            // Tuned defaults (empirical, small-batch): slightly wider preferred band and
            // slightly less aggressive down-weighting while keeping a strong extreme penalty.
            legacy_bpm_preferred_min: 72.0,
            legacy_bpm_preferred_max: 168.0,
            legacy_bpm_soft_min: 60.0,
            legacy_bpm_soft_max: 210.0,
            legacy_bpm_conf_mul_preferred: 1.30,
            legacy_bpm_conf_mul_soft: 0.70,
            legacy_bpm_conf_mul_extreme: 0.01,
            min_bpm: 40.0,  // Lowered from 60.0 to catch slower tracks (ballads, ambient, etc.)
            max_bpm: 240.0, // Raised from 180.0 to catch high-tempo tracks (drum & bass, etc.)
            bpm_resolution: 1.0,
            frame_size: 2048,
            hop_size: 512,
            center_frequency: 440.0,
            soft_chroma_mapping: true,
            soft_mapping_sigma: 0.5,
            chroma_sharpening_power: 1.0, // No sharpening by default (can be enabled with 1.5-2.0)
            enable_key_spectrogram_time_smoothing: true,
            key_spectrogram_smooth_margin: 12,
            enable_key_frame_weighting: true,
            // Default: do not hard-gate frames by tonalness; use soft weighting instead.
            key_min_tonalness: 0.0,
            key_tonalness_power: 2.0,
            key_energy_power: 0.50,
            enable_key_harmonic_mask: true,
            key_harmonic_mask_power: 2.0,
            // Default: off. HPSS median filtering is more expensive than the cheap harmonic mask.
            // Enable via CLI/validation when experimenting.
            enable_key_hpss_harmonic: false,
            key_hpss_frame_step: 4,
            key_hpss_time_margin: 8,
            key_hpss_freq_margin: 8,
            key_hpss_mask_power: 2.0,
            enable_key_stft_override: true,
            key_stft_frame_size: 8192,
            key_stft_hop_size: 512,
            enable_key_log_frequency: false,
            enable_key_beat_synchronous: false,
            enable_key_multi_scale: false,
            key_multi_scale_lengths: vec![120, 360, 720], // ~2s, 6s, 12s at typical frame rates
            key_multi_scale_hop: 60, // ~1s
            key_multi_scale_min_clarity: 0.20,
            key_multi_scale_weights: vec![], // Equal weights by default
            key_template_set: TemplateSet::KrumhanslKessler,
            enable_key_ensemble: false,
            key_ensemble_kk_weight: 0.5,
            key_ensemble_temperley_weight: 0.5,
            enable_key_median: false,
            key_median_segment_length_frames: 480, // ~4 seconds at typical frame rates
            key_median_segment_hop_frames: 120, // ~1 second
            key_median_min_segments: 3,
            // Default: off. Tuning estimation can be unstable on real-world mixes without a more
            // peak/partial-aware frontend (HPCP/CQT). Keep available for experimentation.
            enable_key_tuning_compensation: false,
            key_tuning_max_abs_semitones: 0.08,
            key_tuning_frame_step: 20,
            key_tuning_peak_rel_threshold: 0.35,
            // Default: off. Hard edge trimming can remove useful harmonic content on some tracks.
            // Prefer harmonic masking + frame weighting; keep edge-trim available for experimentation.
            enable_key_edge_trim: false,
            key_edge_trim_fraction: 0.15,
            enable_key_segment_voting: true,
            key_segment_len_frames: 1024,
            key_segment_hop_frames: 512,
            key_segment_min_clarity: 0.20,
            enable_key_mode_heuristic: false,
            // NOTE: Aggressive defaults for Phase 1F DJ validation: minor keys were frequently
            // predicted as major. Keep these tunable via CLI/validation.
            key_mode_third_ratio_margin: 0.00,
            key_mode_flip_min_score_ratio: 0.60,
            enable_key_hpcp: true,
            key_hpcp_peaks_per_frame: 24,
            key_hpcp_num_harmonics: 4,
            key_hpcp_harmonic_decay: 0.60,
            key_hpcp_mag_power: 0.50,
            enable_key_hpcp_whitening: false,
            key_hpcp_whitening_smooth_bins: 31,
            // Experimental: tonic reinforcement can backfire if the bass is not stably pitched.
            enable_key_hpcp_bass_blend: false,
            key_hpcp_bass_fmin_hz: 55.0,
            key_hpcp_bass_fmax_hz: 300.0,
            key_hpcp_bass_weight: 0.35,
            // Experimental: can easily over-bias the result on real-world mixes.
            enable_key_minor_harmonic_bonus: false,
            key_minor_leading_tone_bonus_weight: 0.2,
            #[cfg(feature = "ml")]
            enable_ml_refinement: false,
        }
    }
}

