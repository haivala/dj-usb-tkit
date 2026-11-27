//! Chroma vector extraction
//!
//! Converts FFT magnitude spectrogram to 12-element chroma vectors.
//!
//! Algorithm:
//! 1. Compute STFT (Short-Time Fourier Transform)
//! 2. Convert frequency bins → semitone classes: `semitone = 12 * log2(freq / 440.0) + 57.0`
//! 3. Sum magnitude across octaves for each semitone class
//! 4. Normalize to L2 unit norm
//!
//! # Reference
//!
//! Müller, M., & Ewert, S. (2010). Chroma Toolbox: MATLAB Implementations for Extracting
//! Variants of Chroma-Based Audio Features. *Proceedings of the International Society for
//! Music Information Retrieval Conference*.
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::chroma::extractor::extract_chroma;
//!
//! let samples = vec![0.0f32; 44100 * 5]; // 5 seconds of audio
//! let chroma_vectors = extract_chroma(&samples, 44100, 2048, 512)?;
//! // chroma_vectors is Vec<Vec<f32>> where each inner Vec is a 12-element chroma vector
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use std::cmp::Ordering;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Reference frequency for semitone calculation (A4)
const A4_FREQ: f32 = 440.0;

/// Semitone offset to map A4 to semitone 57 (middle of piano range)
const SEMITONE_OFFSET: f32 = 57.0;

/// Default band-pass limits for chroma extraction.
///
/// Rationale: Very low frequencies are dominated by bass/kick energy (often weakly pitched),
/// and very high frequencies are dominated by broadband/percussive content. Band-limiting
/// typically improves tonal features like chroma/HPCP in real-world mixes.
const DEFAULT_CHROMA_FMIN_HZ: f32 = 100.0;
const DEFAULT_CHROMA_FMAX_HZ: f32 = 5000.0;

/// Estimate a global tuning offset (in semitones) from a magnitude spectrogram.
///
/// Many real-world mixes are slightly detuned vs. A4=440Hz (playback decks, mastering, vintage samples).
/// Without compensation, pitch-class mapping can smear and systematically bias key detection.
///
/// Method (lightweight, HPSS/HPCP-inspired):
/// - For each strong spectral bin, compute the fractional semitone deviation relative to the nearest
///   equal-tempered semitone: \(r = s - round(s)\), where \(s = 12 \log_2(f/440) + 57\).
/// - Convert residuals to angles on the unit circle and compute the weighted circular mean.
/// - The resulting mean residual \(\\delta\\in[-0.5,0.5)\) is the detuning (in semitones) that should
///   be subtracted from semitone positions before pitch-class mapping.
///
/// References / background:
/// - Müller & Ewert (2010): chroma feature variants and robustness considerations (see `docs/literature/mueller_2010_chroma_tutorial.md`).
/// - Tuning-aware pitch-class profile features (HPCP) are commonly described in the MIR literature
///   (we keep this implementation intentionally simple to avoid adding a CQT/CQT-like frontend).
pub fn estimate_tuning_offset_semitones_from_spectrogram(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    fmin_hz: f32,
    fmax_hz: f32,
    frame_step: usize,
    peak_rel_threshold: f32,
) -> Result<f32, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(0.0);
    }
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                f.len()
            )));
        }
    }

    if sample_rate == 0 || fft_size == 0 {
        return Ok(0.0);
    }

    let freq_resolution = sample_rate as f32 / fft_size as f32;
    let fmin = fmin_hz.max(20.0);
    let fmax = fmax_hz.max(fmin + 1.0).min(sample_rate as f32 / 2.0);
    let step = frame_step.max(1);
    let thr = peak_rel_threshold.clamp(0.0, 1.0);

    let mut sum_sin = 0.0f32;
    let mut sum_cos = 0.0f32;
    let mut sum_w = 0.0f32;

    for (t, frame) in magnitude_spec_frames.iter().enumerate().step_by(step) {
        // Find the frame peak in-band to set a relative threshold.
        let mut peak = 0.0f32;
        for (bin_idx, &mag) in frame.iter().enumerate() {
            let freq = bin_idx as f32 * freq_resolution;
            if freq < fmin {
                continue;
            }
            if freq > fmax {
                break;
            }
            peak = peak.max(mag);
        }
        if peak <= 1e-12 {
            continue;
        }
        let abs_thr = peak * thr;

        for (bin_idx, &mag) in frame.iter().enumerate() {
            if mag < abs_thr {
                continue;
            }
            let freq = bin_idx as f32 * freq_resolution;
            if freq < fmin {
                continue;
            }
            if freq > fmax {
                break;
            }
            // Compute semitone position, then residual to nearest semitone.
            let semitone = 12.0 * (freq / A4_FREQ).log2() + SEMITONE_OFFSET;
            let residual = semitone - semitone.round(); // ≈ [-0.5, 0.5]

            // Weight: mild compression (avoid a few bins dominating).
            let w = mag.max(0.0).powf(0.5);
            if w <= 0.0 {
                continue;
            }
            let angle = 2.0 * std::f32::consts::PI * residual;
            sum_sin += w * angle.sin();
            sum_cos += w * angle.cos();
            sum_w += w;
        }

        // Avoid pathological long loops if something is wrong.
        let _ = t;
    }

    if sum_w <= 1e-6 {
        return Ok(0.0);
    }

    let r = (sum_sin * sum_sin + sum_cos * sum_cos).sqrt() / sum_w;
    if r < 0.05 {
        // Residuals are not concentrated; return no correction.
        return Ok(0.0);
    }

    let mean_angle = sum_sin.atan2(sum_cos);
    let delta = mean_angle / (2.0 * std::f32::consts::PI); // [-0.5, 0.5)
    Ok(delta)
}

/// Extract chroma vectors from audio samples
///
/// Computes STFT, maps frequencies to semitone classes, and sums across octaves
/// to produce 12-element chroma vectors (one per frame).
///
/// # Arguments
///
/// * `samples` - Audio samples (mono, normalized to [-1.0, 1.0])
/// * `sample_rate` - Sample rate in Hz (typically 44100 or 48000)
/// * `frame_size` - FFT frame size (default: 2048)
/// * `hop_size` - Hop size between frames (default: 512)
///
/// # Returns
///
/// Vector of 12-element chroma vectors (one per frame)
/// Each chroma vector represents the pitch-class distribution for that frame
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Samples are empty
/// - Invalid parameters (frame_size or hop_size == 0)
/// - STFT computation fails
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::chroma::extractor::extract_chroma;
///
/// let samples = vec![0.0f32; 44100 * 5]; // 5 seconds
/// let chroma_vectors = extract_chroma(&samples, 44100, 2048, 512)?;
///
/// // Each vector has 12 elements (one per semitone class: C, C#, D, ..., B)
/// assert_eq!(chroma_vectors[0].len(), 12);
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn extract_chroma(
    samples: &[f32],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    extract_chroma_with_options(samples, sample_rate, frame_size, hop_size, true, 0.5)
}

/// Extract chroma vectors with configurable options
///
/// Same as `extract_chroma` but allows configuration of soft mapping.
pub fn extract_chroma_with_options(
    samples: &[f32],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    log::debug!("Extracting chroma: {} samples at {} Hz, frame_size={}, hop_size={}, soft_mapping={}",
                samples.len(), sample_rate, frame_size, hop_size, soft_mapping);
    
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput("Empty audio samples".to_string()));
    }
    
    if frame_size == 0 {
        return Err(AnalysisError::InvalidInput("Frame size must be > 0".to_string()));
    }
    
    if hop_size == 0 {
        return Err(AnalysisError::InvalidInput("Hop size must be > 0".to_string()));
    }
    
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
    }
    
    // Step 1: Compute STFT
    let stft_magnitudes = compute_stft(samples, frame_size, hop_size)?;
    
    if stft_magnitudes.is_empty() {
        return Ok(vec![]);
    }
    
    // Step 2: Convert each frame to chroma vector
    let mut chroma_vectors = Vec::with_capacity(stft_magnitudes.len());
    
    for frame in &stft_magnitudes {
        let chroma = frame_to_chroma(frame, sample_rate, frame_size, soft_mapping, soft_mapping_sigma)?;
        chroma_vectors.push(chroma);
    }
    
    log::debug!("Extracted {} chroma vectors", chroma_vectors.len());
    
    Ok(chroma_vectors)
}

/// Compute STFT (Short-Time Fourier Transform)
///
/// Computes magnitude spectrogram using windowed FFT.
///
/// # Arguments
///
/// * `samples` - Audio samples
/// * `frame_size` - FFT frame size (window size)
/// * `hop_size` - Hop size between frames
///
/// # Returns
///
/// Vector of magnitude spectra (one per frame)
/// Each inner vector has `frame_size / 2 + 1` frequency bins
pub fn compute_stft(
    samples: &[f32],
    frame_size: usize,
    hop_size: usize,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    let n_samples = samples.len();
    
    if n_samples < frame_size {
        // Not enough samples for even one frame
        return Ok(vec![]);
    }
    
    // Compute number of frames
    let n_frames = (n_samples - frame_size) / hop_size + 1;
    let mut magnitudes = Vec::with_capacity(n_frames);
    
    // Create Hann window
    let window: Vec<f32> = (0..frame_size)
        .map(|i| {
            let x = 2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32;
            0.5 * (1.0 - x.cos())
        })
        .collect();
    
    // FFT planner
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame_size);
    
    // Process each frame
    for frame_idx in 0..n_frames {
        let start = frame_idx * hop_size;
        let end = start + frame_size;
        
        if end > n_samples {
            break;
        }
        
        // Window the frame
        let mut fft_input: Vec<Complex<f32>> = samples[start..end]
            .iter()
            .zip(window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();
        
        // Forward FFT
        fft.process(&mut fft_input);
        
        // Compute magnitude spectrum (only need first frame_size/2 + 1 bins for real FFT)
        let n_bins = frame_size / 2 + 1;
        let magnitude: Vec<f32> = fft_input[..n_bins]
            .iter()
            .map(|x| (x.re * x.re + x.im * x.im).sqrt())
            .collect();
        
        magnitudes.push(magnitude);
    }
    
    Ok(magnitudes)
}

/// Convert a single FFT magnitude frame to chroma vector
///
/// Maps frequency bins to semitone classes and sums across octaves.
///
/// # Arguments
///
/// * `magnitude_frame` - FFT magnitude spectrum for one frame
/// * `sample_rate` - Sample rate in Hz
/// * `fft_size` - FFT size (same as frame_size)
/// * `soft_mapping` - Enable soft mapping (spread to neighboring semitones)
/// * `soft_mapping_sigma` - Standard deviation for soft mapping (in semitones)
///
/// # Returns
///
/// 12-element chroma vector (one per semitone class)
fn frame_to_chroma(
    magnitude_frame: &[f32],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
) -> Result<Vec<f32>, AnalysisError> {
    frame_to_chroma_tuned(
        magnitude_frame,
        sample_rate,
        fft_size,
        soft_mapping,
        soft_mapping_sigma,
        0.0,
    )
}

fn frame_to_chroma_tuned(
    magnitude_frame: &[f32],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
) -> Result<Vec<f32>, AnalysisError> {
    // Initialize chroma vector (12 semitone classes)
    let mut chroma = vec![0.0f32; 12];
    
    // Frequency resolution: sample_rate / fft_size
    let freq_resolution = sample_rate as f32 / fft_size as f32;
    
    // Process each frequency bin
    for (bin_idx, &magnitude) in magnitude_frame.iter().enumerate() {
        // Convert bin index to frequency
        let freq = bin_idx as f32 * freq_resolution;
        
        // Band-limit for tonal/chroma extraction.
        if freq < DEFAULT_CHROMA_FMIN_HZ {
            continue;
        }
        if freq > DEFAULT_CHROMA_FMAX_HZ.min(sample_rate as f32 / 2.0) {
            break;
        }
        
        // Skip Nyquist and above
        if freq >= sample_rate as f32 / 2.0 {
            break;
        }
        
        // Convert frequency to semitone (and apply optional tuning compensation).
        // Formula: semitone = 12 * log2(freq / 440.0) + 57.0
        let semitone = 12.0 * (freq / A4_FREQ).log2() + SEMITONE_OFFSET - tuning_offset_semitones;
        
        // Light magnitude compression to reduce dominance of broadband energy.
        // (Key/chroma benefits from compressing dynamic range in real-world mixes.)
        //
        // Note: We intentionally do **not** apply additional frequency-dependent weighting here.
        // In early real-world tests on DJ tracks, even mild inverse-frequency weighting increased
        // low-frequency bias and reduced key accuracy.
        let magnitude = magnitude.max(0.0).powf(0.6);
        let contrib = magnitude;

        if soft_mapping {
            // Soft mapping: spread magnitude to neighboring semitone classes using a Gaussian kernel
            // in **circular** pitch-class space.
            //
            // NOTE: We must treat distance on the 12-tone circle (wrap-around between 11 and 0).
            // A linear distance would incorrectly zero out energy near the boundary (e.g., B↔C),
            // biasing chroma statistics and downstream key detection.
            let semitone_pc = semitone.rem_euclid(12.0); // [0, 12)
            let primary_pc = semitone_pc.round().rem_euclid(12.0); // [0, 12)
            let primary_class = primary_pc as i32;

            // Compute weights for the nearest pitch class and its immediate neighbors.
            for offset in -1..=1 {
                let target_class = (primary_class + offset).rem_euclid(12);
                let target_pc = target_class as f32; // [0, 11]
                let mut distance = (semitone_pc - target_pc).abs();
                distance = distance.min(12.0 - distance); // circular distance

                // Gaussian weight in semitone units.
                let sigma = soft_mapping_sigma.max(1e-6);
                let weight = (-distance * distance / (2.0 * sigma * sigma)).exp();

                chroma[target_class as usize] += contrib * weight;
            }
        } else {
            // Hard assignment: assign to nearest semitone class
            let semitone_class = (semitone.round() as i32) % 12;
            // Handle negative modulo
            let semitone_class = if semitone_class < 0 {
                semitone_class + 12
            } else {
                semitone_class
            } as usize;
            
            // Sum magnitude across octaves for this semitone class
            chroma[semitone_class] += contrib;
        }
    }
    
    // L2 normalize chroma vector
    let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
    
    if norm > EPSILON {
        for x in &mut chroma {
            *x /= norm;
        }
    }
    
    Ok(chroma)
}

/// Convert a single FFT magnitude frame to an HPCP-style pitch-class profile (12 bins).
///
/// Compared to raw chroma accumulation, this method:
/// - Uses simple spectral peak picking (local maxima) to reduce broadband/percussive influence.
/// - Spreads each peak across a few harmonics to stabilize tonal cues.
///
/// This is a lightweight approximation intended for real-world mixes (DJ tracks), while still using
/// the existing STFT front-end (no CQT required).
fn frame_to_hpcp_tuned(
    magnitude_frame: &[f32],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
    peaks_per_frame: usize,
    num_harmonics: usize,
    harmonic_decay: f32,
    mag_power: f32,
    enable_whitening: bool,
    whitening_smooth_bins: usize,
) -> Result<Vec<f32>, AnalysisError> {
    frame_to_hpcp_tuned_band(
        magnitude_frame,
        sample_rate,
        fft_size,
        soft_mapping_sigma,
        tuning_offset_semitones,
        peaks_per_frame,
        num_harmonics,
        harmonic_decay,
        mag_power,
        enable_whitening,
        whitening_smooth_bins,
        DEFAULT_CHROMA_FMIN_HZ,
        DEFAULT_CHROMA_FMAX_HZ,
    )
}

fn frame_to_hpcp_tuned_band(
    magnitude_frame: &[f32],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
    peaks_per_frame: usize,
    num_harmonics: usize,
    harmonic_decay: f32,
    mag_power: f32,
    enable_whitening: bool,
    whitening_smooth_bins: usize,
    fmin_hz: f32,
    fmax_hz: f32,
) -> Result<Vec<f32>, AnalysisError> {
    let mut pc = vec![0.0f32; 12];
    if magnitude_frame.is_empty() || sample_rate == 0 || fft_size == 0 {
        return Ok(pc);
    }

    let freq_resolution = sample_rate as f32 / fft_size as f32;
    let fmin = fmin_hz.max(20.0);
    let fmax = fmax_hz.min(sample_rate as f32 / 2.0);
    if fmax <= fmin {
        return Ok(pc);
    }

    // Optional per-frame spectral whitening (timbre suppression).
    //
    // Uses a simple moving-average smoother over frequency bins, then divides magnitude by the
    // local mean. This boosts narrowband peaks relative to broadband/timbral coloration and tends
    // to make peak picking more robust on real-world mixes.
    let mut whitened: Vec<f32> = Vec::new();
    let use_whitening = enable_whitening && whitening_smooth_bins >= 3;
    if use_whitening {
        let win = whitening_smooth_bins.max(3) | 1; // force odd window
        let half = win / 2;
        let mut prefix = vec![0.0f32; magnitude_frame.len() + 1];
        for (i, &x) in magnitude_frame.iter().enumerate() {
            prefix[i + 1] = prefix[i] + x.max(0.0);
        }
        whitened.resize(magnitude_frame.len(), 0.0);
        let eps = 1e-12f32;
        for i in 0..magnitude_frame.len() {
            let l = i.saturating_sub(half);
            let r = (i + half).min(magnitude_frame.len().saturating_sub(1));
            let denom = (r + 1 - l) as f32;
            let mean = (prefix[r + 1] - prefix[l]) / denom.max(1.0);
            let v = magnitude_frame[i].max(0.0) / (mean + eps);
            whitened[i] = v.min(20.0);
        }
    }

    // 1) Collect local maxima as peak candidates (in-band).
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    for bin in 1..magnitude_frame.len().saturating_sub(1) {
        let freq = bin as f32 * freq_resolution;
        if freq < fmin {
            continue;
        }
        if freq > fmax {
            break;
        }
        let m = if use_whitening { whitened[bin] } else { magnitude_frame[bin] };
        let m_prev = if use_whitening { whitened[bin - 1] } else { magnitude_frame[bin - 1] };
        let m_next = if use_whitening { whitened[bin + 1] } else { magnitude_frame[bin + 1] };
        if m <= m_prev || m < m_next {
            continue;
        }
        peaks.push((bin, m));
    }

    if peaks.is_empty() {
        return Ok(pc);
    }

    // Keep top-K peaks by magnitude.
    let k = peaks_per_frame.max(1).min(peaks.len());
    peaks.select_nth_unstable_by(k - 1, |a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    peaks.truncate(k);

    let sigma = soft_mapping_sigma.max(1e-6);
    let hmax = num_harmonics.max(1);
    let decay = harmonic_decay.clamp(0.0, 1.0);
    let p = mag_power.clamp(0.05, 1.0);

    // 2) Add peak contributions across harmonics.
    //
    // If whitening is enabled, we only use it to *select* peaks (robust local-max detection and
    // ranking), but we still weight peak strength using the original magnitudes. This avoids
    // over-amplifying noise/partials in sparse regions and keeps the weighting tied to actual energy.
    for (bin, _score) in peaks {
        let f0 = bin as f32 * freq_resolution;
        if f0 <= 0.0 {
            continue;
        }
        let w0 = magnitude_frame[bin].max(0.0).powf(p);
        if w0 <= 0.0 {
            continue;
        }

        for h in 1..=hmax {
            let fh = f0 * (h as f32);
            if fh > fmax {
                break;
            }
            if fh < fmin {
                continue;
            }

            let semitone = 12.0 * (fh / A4_FREQ).log2() + SEMITONE_OFFSET - tuning_offset_semitones;
            let semitone_pc = semitone.rem_euclid(12.0);
            let primary_pc = semitone_pc.round().rem_euclid(12.0);
            let primary_class = primary_pc as i32;

            let hw = (decay.powi((h as i32) - 1)) / (h as f32);
            let contrib = w0 * hw;

            for offset in -1..=1 {
                let target_class = (primary_class + offset).rem_euclid(12);
                let target_pc = target_class as f32;
                let mut distance = (semitone_pc - target_pc).abs();
                distance = distance.min(12.0 - distance);
                let weight = (-distance * distance / (2.0 * sigma * sigma)).exp();
                pc[target_class as usize] += contrib * weight;
            }
        }
    }

    // Normalize to unit norm.
    let norm: f32 = pc.iter().map(|&x| x * x).sum::<f32>().sqrt();
    if norm > EPSILON {
        for x in pc.iter_mut() {
            *x /= norm;
        }
    }
    Ok(pc)
}

/// Convert a linear STFT magnitude spectrogram to a log-frequency (semitone-aligned) spectrogram.
///
/// This resamples the linear frequency bins into semitone-aligned bins, providing better pitch-class
/// resolution for key detection. Each output bin corresponds to one semitone, and magnitudes are
/// accumulated using linear interpolation across the frequency range covered by each semitone.
///
/// # Arguments
///
/// * `linear_spec_frames` - Linear STFT magnitude spectrogram (frames × frequency bins)
/// * `sample_rate` - Sample rate in Hz
/// * `fft_size` - FFT size used for the linear STFT
/// * `fmin_hz` - Minimum frequency to include (Hz)
/// * `fmax_hz` - Maximum frequency to include (Hz)
/// * `semitones_per_octave` - Number of semitones per octave (default: 12)
///
/// # Returns
///
/// Log-frequency spectrogram (frames × semitone bins), where each bin corresponds to one semitone.
/// The semitone bins are indexed from 0 (lowest semitone in range) to N-1 (highest semitone in range).
pub fn convert_linear_to_log_frequency_spectrogram(
    linear_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    fmin_hz: f32,
    fmax_hz: f32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if linear_spec_frames.is_empty() {
        return Ok(vec![]);
    }
    
    let n_frames = linear_spec_frames.len();
    let n_linear_bins = linear_spec_frames[0].len();
    
    // Validate all frames have the same length
    for (i, frame) in linear_spec_frames.iter().enumerate() {
        if frame.len() != n_linear_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_linear_bins, i, frame.len()
            )));
        }
    }
    
    if sample_rate == 0 || fft_size == 0 {
        return Err(AnalysisError::InvalidInput(
            "Sample rate and FFT size must be > 0".to_string()
        ));
    }
    
    let freq_resolution = sample_rate as f32 / fft_size as f32;
    let nyquist = sample_rate as f32 / 2.0;
    let fmin = fmin_hz.max(20.0);
    let fmax = fmax_hz.min(nyquist - 1.0);
    
    // Compute semitone range: convert frequency bounds to semitones
    // semitone = 12 * log2(freq / 440.0) + 57.0
    let semitone_min = 12.0 * (fmin / A4_FREQ).log2() + SEMITONE_OFFSET;
    let semitone_max = 12.0 * (fmax / A4_FREQ).log2() + SEMITONE_OFFSET;
    
    // Round to nearest semitone and create bin range
    let semitone_bin_min = semitone_min.floor() as i32;
    let semitone_bin_max = semitone_max.ceil() as i32;
    let n_semitone_bins = (semitone_bin_max - semitone_bin_min + 1) as usize;
    
    if n_semitone_bins == 0 {
        return Err(AnalysisError::InvalidInput(
            "Invalid semitone range: fmin >= fmax".to_string()
        ));
    }
    
    // Precompute semitone bin frequency bounds (for reference, not used in conversion)
    // Note: We use direct interpolation in the conversion loop below, so this is just for documentation
    let _semitone_freqs: Vec<(f32, f32)> = (0..n_semitone_bins)
        .map(|bin_idx| {
            let semitone = (semitone_bin_min + bin_idx as i32) as f32;
            // Semitone bin boundaries: ±0.5 semitones around center
            let freq_low = A4_FREQ * 2.0_f32.powf((semitone - 0.5 - SEMITONE_OFFSET) / 12.0);
            let freq_high = A4_FREQ * 2.0_f32.powf((semitone + 0.5 - SEMITONE_OFFSET) / 12.0);
            (freq_low.max(fmin), freq_high.min(fmax))
        })
        .collect();
    
    // Convert each frame
    let mut log_freq_spec = Vec::with_capacity(n_frames);
    
    for frame_idx in 0..n_frames {
        let linear_frame = &linear_spec_frames[frame_idx];
        let mut log_frame = vec![0.0f32; n_semitone_bins];
        
        // For each linear frequency bin, accumulate into semitone bins using linear interpolation
        for (linear_bin, &magnitude) in linear_frame.iter().enumerate() {
            if magnitude <= 0.0 {
                continue;
            }
            
            let freq = linear_bin as f32 * freq_resolution;
            if freq < fmin || freq >= fmax || freq >= nyquist {
                continue;
            }
            
            // Find which semitone bin(s) this frequency belongs to
            let semitone = 12.0 * (freq / A4_FREQ).log2() + SEMITONE_OFFSET;
            let semitone_float = semitone - semitone_bin_min as f32;
            
            // Linear interpolation: distribute magnitude to neighboring semitone bins
            let bin_idx_float = semitone_float;
            let bin_idx_low = bin_idx_float.floor() as usize;
            let bin_idx_high = (bin_idx_float.ceil() as usize).min(n_semitone_bins - 1);
            
            if bin_idx_low < n_semitone_bins {
                let weight_high = bin_idx_float - bin_idx_low as f32;
                let weight_low = 1.0 - weight_high;
                
                log_frame[bin_idx_low] += magnitude * weight_low;
                if bin_idx_high != bin_idx_low && bin_idx_high < n_semitone_bins {
                    log_frame[bin_idx_high] += magnitude * weight_high;
                }
            }
        }
        
        log_freq_spec.push(log_frame);
    }
    
    Ok(log_freq_spec)
}

/// Extract beat-synchronous chroma vectors from a magnitude spectrogram.
///
/// This aligns chroma extraction to beat boundaries instead of fixed-time frames, improving
/// harmonic coherence by aligning to musical structure. For each beat interval, chroma vectors
/// from all STFT frames within that interval are averaged.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - Linear STFT magnitude spectrogram (frames × frequency bins)
/// * `sample_rate` - Sample rate in Hz
/// * `fft_size` - FFT size used for the spectrogram
/// * `hop_size` - Hop size used for the spectrogram (samples between frames)
/// * `beat_times` - Beat times in seconds (must be sorted, ascending)
/// * `soft_mapping` - Enable soft mapping (spread to neighboring semitones)
/// * `soft_mapping_sigma` - Standard deviation for soft mapping (in semitones)
/// * `tuning_offset_semitones` - Optional tuning offset to apply (in semitones)
///
/// # Returns
///
/// Chroma vectors (one per beat interval) and per-beat energies
pub fn extract_beat_synchronous_chroma(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    hop_size: usize,
    beat_times: &[f32],
    soft_mapping: bool,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
) -> Result<(Vec<Vec<f32>>, Vec<f32>), AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    
    if beat_times.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Beat-synchronous chroma requires at least one beat time".to_string()
        ));
    }
    
    // Compute frame times (in seconds)
    let frame_duration = hop_size as f32 / sample_rate as f32;
    let frame_times: Vec<f32> = (0..magnitude_spec_frames.len())
        .map(|i| i as f32 * frame_duration)
        .collect();
    
    // For each beat interval, collect chroma vectors and average them
    let mut beat_chroma_vectors = Vec::with_capacity(beat_times.len().saturating_sub(1));
    let mut beat_energies = Vec::with_capacity(beat_times.len().saturating_sub(1));
    
    for i in 0..beat_times.len().saturating_sub(1) {
        let beat_start = beat_times[i];
        let beat_end = beat_times[i + 1];
        
        // Find all frames that fall within this beat interval
        let mut interval_chroma = Vec::new();
        let mut interval_energy = 0.0f32;
        
        for (frame_idx, &frame_time) in frame_times.iter().enumerate() {
            // Include frames that start within the beat interval
            // (frames that start before beat_start but extend into it are also included)
            if frame_time >= beat_start && frame_time < beat_end {
                let chroma = frame_to_chroma_tuned(
                    &magnitude_spec_frames[frame_idx],
                    sample_rate,
                    fft_size,
                    soft_mapping,
                    soft_mapping_sigma,
                    tuning_offset_semitones,
                )?;
                
                let energy: f32 = magnitude_spec_frames[frame_idx]
                    .iter()
                    .map(|&x| x * x)
                    .sum();
                
                interval_chroma.push(chroma);
                interval_energy += energy;
            }
        }
        
        // Average chroma vectors within the beat interval
        if !interval_chroma.is_empty() {
            let mut avg_chroma = vec![0.0f32; 12];
            for chroma in &interval_chroma {
                for (j, &val) in chroma.iter().enumerate() {
                    avg_chroma[j] += val;
                }
            }
            let n = interval_chroma.len() as f32;
            for val in &mut avg_chroma {
                *val /= n;
            }
            
            // Re-normalize to unit L2 norm
            let norm: f32 = avg_chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
            if norm > EPSILON {
                for val in &mut avg_chroma {
                    *val /= norm;
                }
            }
            
            beat_chroma_vectors.push(avg_chroma);
            beat_energies.push(interval_energy);
        } else {
            // Empty beat interval - use zero chroma
            beat_chroma_vectors.push(vec![0.0f32; 12]);
            beat_energies.push(0.0);
        }
    }
    
    Ok((beat_chroma_vectors, beat_energies))
}

/// Extract chroma vectors from a log-frequency (semitone-aligned) spectrogram.
///
/// This is optimized for log-frequency spectrograms where each bin corresponds to one semitone.
/// Chroma is computed by summing magnitudes across octaves (mod 12).
///
/// # Arguments
///
/// * `log_freq_spec_frames` - Log-frequency spectrogram (frames × semitone bins)
/// * `semitone_offset` - Semitone offset of the first bin (e.g., 0 for C0, 12 for C1, etc.)
///
/// # Returns
///
/// Chroma vectors (frames × 12 pitch classes)
pub fn extract_chroma_from_log_frequency_spectrogram(
    log_freq_spec_frames: &[Vec<f32>],
    semitone_offset: i32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if log_freq_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut chroma_vectors = Vec::with_capacity(log_freq_spec_frames.len());
    
    for frame in log_freq_spec_frames {
        let mut chroma = vec![0.0f32; 12];
        
        // Each bin in the log-frequency spectrogram corresponds to one semitone
        // Sum across octaves (mod 12) to get chroma
        for (bin_idx, &magnitude) in frame.iter().enumerate() {
            if magnitude <= 0.0 {
                continue;
            }
            
            // Compute which pitch class (0-11) this semitone bin belongs to
            let semitone = semitone_offset + bin_idx as i32;
            let pitch_class = semitone.rem_euclid(12);
            let pc_idx = if pitch_class < 0 {
                pitch_class + 12
            } else {
                pitch_class
            } as usize;
            
            chroma[pc_idx] += magnitude;
        }
        
        // L2 normalize
        let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
        if norm > EPSILON {
            for x in &mut chroma {
                *x /= norm;
            }
        }
        
        chroma_vectors.push(chroma);
    }
    
    Ok(chroma_vectors)
}

/// Extract chroma vectors from a precomputed magnitude spectrogram.
///
/// This is equivalent to `extract_chroma_with_options`, but avoids recomputing the STFT when the
/// caller already has magnitude frames (e.g., tempogram pipeline).
pub fn extract_chroma_from_spectrogram_with_options(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                f.len()
            )));
        }
    }

    let mut chroma_vectors = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        chroma_vectors.push(frame_to_chroma(
            frame,
            sample_rate,
            fft_size,
            soft_mapping,
            soft_mapping_sigma,
        )?);
    }
    Ok(chroma_vectors)
}

/// Extract chroma vectors and per-frame energy (sum of squared magnitudes) from a spectrogram.
pub fn extract_chroma_from_spectrogram_with_options_and_energy(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
) -> Result<(Vec<Vec<f32>>, Vec<f32>), AnalysisError> {
    extract_chroma_from_spectrogram_with_options_and_energy_tuned(
        magnitude_spec_frames,
        sample_rate,
        fft_size,
        soft_mapping,
        soft_mapping_sigma,
        0.0,
    )
}

/// Like `extract_chroma_from_spectrogram_with_options_and_energy`, but applies a per-track tuning offset
/// (in semitones) before mapping to pitch classes.
pub fn extract_chroma_from_spectrogram_with_options_and_energy_tuned(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping: bool,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
) -> Result<(Vec<Vec<f32>>, Vec<f32>), AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                f.len()
            )));
        }
    }

    let mut chroma_vectors = Vec::with_capacity(magnitude_spec_frames.len());
    let mut energies = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        let e: f32 = frame.iter().map(|&x| x * x).sum();
        energies.push(e);
        chroma_vectors.push(frame_to_chroma_tuned(
            frame,
            sample_rate,
            fft_size,
            soft_mapping,
            soft_mapping_sigma,
            tuning_offset_semitones,
        )?);
    }
    Ok((chroma_vectors, energies))
}

/// Extract HPCP-style pitch-class profiles and per-frame energy from a spectrogram.
///
/// This is a key-only feature option (more robust on real-world mixes than raw chroma).
pub fn extract_hpcp_from_spectrogram_with_options_and_energy_tuned(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
    peaks_per_frame: usize,
    num_harmonics: usize,
    harmonic_decay: f32,
    mag_power: f32,
    enable_whitening: bool,
    whitening_smooth_bins: usize,
) -> Result<(Vec<Vec<f32>>, Vec<f32>), AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins, i, f.len()
            )));
        }
    }

    let mut pcs = Vec::with_capacity(magnitude_spec_frames.len());
    let mut energies = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        let e: f32 = frame.iter().map(|&x| x * x).sum();
        energies.push(e);
        pcs.push(frame_to_hpcp_tuned(
            frame,
            sample_rate,
            fft_size,
            soft_mapping_sigma,
            tuning_offset_semitones,
            peaks_per_frame,
            num_harmonics,
            harmonic_decay,
            mag_power,
            enable_whitening,
            whitening_smooth_bins,
        )?);
    }
    Ok((pcs, energies))
}

/// Extract blended HPCP profiles: (1-w)*full_band + w*bass_band, normalized per frame.
pub fn extract_hpcp_bass_blend_from_spectrogram_with_options_and_energy_tuned(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    soft_mapping_sigma: f32,
    tuning_offset_semitones: f32,
    peaks_per_frame: usize,
    num_harmonics: usize,
    harmonic_decay: f32,
    mag_power: f32,
    enable_whitening: bool,
    whitening_smooth_bins: usize,
    bass_fmin_hz: f32,
    bass_fmax_hz: f32,
    bass_weight: f32,
) -> Result<(Vec<Vec<f32>>, Vec<f32>), AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins, i, f.len()
            )));
        }
    }

    let w = bass_weight.clamp(0.0, 1.0);
    let mut pcs = Vec::with_capacity(magnitude_spec_frames.len());
    let mut energies = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        let e: f32 = frame.iter().map(|&x| x * x).sum();
        energies.push(e);

        let full = frame_to_hpcp_tuned(
            frame,
            sample_rate,
            fft_size,
            soft_mapping_sigma,
            tuning_offset_semitones,
            peaks_per_frame,
            num_harmonics,
            harmonic_decay,
            mag_power,
            enable_whitening,
            whitening_smooth_bins,
        )?;
        let bass = frame_to_hpcp_tuned_band(
            frame,
            sample_rate,
            fft_size,
            soft_mapping_sigma,
            tuning_offset_semitones,
            peaks_per_frame.min(12).max(1),
            num_harmonics,
            harmonic_decay,
            mag_power,
            enable_whitening,
            whitening_smooth_bins,
            bass_fmin_hz,
            bass_fmax_hz,
        )?;

        let mut blend = vec![0.0f32; 12];
        for i in 0..12 {
            blend[i] = (1.0 - w) * full[i] + w * bass[i];
        }
        let norm: f32 = blend.iter().map(|&x| x * x).sum::<f32>().sqrt();
        if norm > 1e-10 {
            for x in blend.iter_mut() {
                *x /= norm;
            }
        }
        pcs.push(blend);
    }
    Ok((pcs, energies))
}

/// Time-smooth a magnitude spectrogram (horizontal smoothing) to suppress percussive transients.
///
/// This is a lightweight percussive-suppression step (HPSS-inspired) used for key detection:
/// percussive components are short-lived in time, while harmonic components are sustained.
/// A moving-average across time attenuates transients and emphasizes sustained energy.
pub fn smooth_spectrogram_time(
    magnitude_spec_frames: &[Vec<f32>],
    margin: usize,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if magnitude_spec_frames.is_empty() || margin == 0 {
        return Ok(magnitude_spec_frames.to_vec());
    }
    let n_frames = magnitude_spec_frames.len();
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                f.len()
            )));
        }
    }

    let mut out = vec![vec![0.0f32; n_bins]; n_frames];

    // For each bin, do a sliding window average over time.
    for bin in 0..n_bins {
        // Prefix sums for O(1) range sum queries.
        let mut prefix = vec![0.0f32; n_frames + 1];
        for t in 0..n_frames {
            prefix[t + 1] = prefix[t] + magnitude_spec_frames[t][bin];
        }
        for t in 0..n_frames {
            let start = t.saturating_sub(margin);
            let end = (t + margin + 1).min(n_frames);
            let sum = prefix[end] - prefix[start];
            let denom = (end - start).max(1) as f32;
            out[t][bin] = sum / denom;
        }
    }

    Ok(out)
}

/// Compute a harmonic-emphasized spectrogram using a cheap time-smoothing-derived soft mask.
///
/// This is an HPSS-inspired approximation intended specifically for **key detection**:
/// - Harmonic content is relatively sustained across time ⇒ preserved by time-smoothing.
/// - Percussive content is transient ⇒ not supported by time-smoothing.
///
/// Algorithm:
/// 1. Compute a time-smoothed spectrogram \(H\_est\).
/// 2. Estimate percussive residual \(P\_est = max(0, X - H\_est)\).
/// 3. Apply a soft mask \(M = H\_est^p / (H\_est^p + P\_est^p)\).
/// 4. Return \(H = X * M\).
///
/// This avoids the heavy iterative median-filter HPSS while still strongly suppressing
/// broadband transients in real-world mixes.
pub fn harmonic_spectrogram_time_mask(
    magnitude_spec_frames: &[Vec<f32>],
    smooth_margin: usize,
    mask_power: f32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    let n_frames = magnitude_spec_frames.len();
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                f.len()
            )));
        }
    }

    let h_est = smooth_spectrogram_time(magnitude_spec_frames, smooth_margin)?;
    let p = mask_power.max(1.0);
    let eps = 1e-12f32;

    let mut out = vec![vec![0.0f32; n_bins]; n_frames];
    for t in 0..n_frames {
        for b in 0..n_bins {
            let x = magnitude_spec_frames[t][b].max(0.0);
            let h = h_est[t][b].max(0.0);
            let r = (x - h).max(0.0);
            let hp = h.powf(p);
            let rp = r.powf(p);
            let m = hp / (hp + rp + eps);
            out[t][b] = x * m;
        }
    }
    Ok(out)
}

/// Compute a harmonic-emphasized spectrogram using median-filter HPSS (time+frequency medians).
///
/// This is intended specifically for **key detection** (key-only path) and is designed to be
/// more "proper HPSS" than `harmonic_spectrogram_time_mask()`, while still being practical:
/// - We **band-limit** to the chroma/HPCP range (default 100–5000 Hz).
/// - We compute medians on a **time-downsampled** spectrogram (`frame_step`) to reduce cost.
/// - We apply the resulting harmonic soft mask back to the full-resolution spectrogram.
///
/// Algorithm (single-pass, median-filter HPSS):
/// 1) Let \(X\) be the magnitude spectrogram (frames × bins).
/// 2) Compute harmonic estimate \(H\_est\) by median-filtering across **time**.
/// 3) Compute percussive estimate \(P\_est\) by median-filtering across **frequency**.
/// 4) Soft mask: \(M\_h = H\_est^p / (H\_est^p + P\_est^p + \epsilon)\).
/// 5) Output: \(H = X \cdot M\_h\).
///
/// Reference:
/// - Driedger, J., & Müller, M. (2014). Extending Harmonic-Percussive Separation of Audio Signals. ISMIR.
pub fn harmonic_spectrogram_hpss_median_mask(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    fft_size: usize,
    fmin_hz: f32,
    fmax_hz: f32,
    frame_step: usize,
    time_margin: usize,
    freq_margin: usize,
    mask_power: f32,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    let n_frames = magnitude_spec_frames.len();
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty spectrogram frames".to_string()));
    }
    for (i, f) in magnitude_spec_frames.iter().enumerate() {
        if f.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent spectrogram frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins, i, f.len()
            )));
        }
    }
    if sample_rate == 0 || fft_size == 0 {
        return Ok(magnitude_spec_frames.to_vec());
    }

    let freq_resolution = sample_rate as f32 / fft_size as f32;
    let fmin = fmin_hz.max(20.0);
    let fmax = fmax_hz.max(fmin + 1.0).min(sample_rate as f32 / 2.0);
    let mut bin_start = (fmin / freq_resolution).floor() as isize;
    let mut bin_end = (fmax / freq_resolution).ceil() as isize;
    bin_start = bin_start.clamp(0, n_bins as isize);
    bin_end = bin_end.clamp(0, n_bins as isize);
    if bin_end <= bin_start {
        return Ok(magnitude_spec_frames.to_vec());
    }
    let bin_start = bin_start as usize;
    let bin_end = bin_end as usize;
    let band_bins = bin_end - bin_start;

    let step = frame_step.max(1);
    let ds_indices: Vec<usize> = (0..n_frames).step_by(step).collect();
    let n_ds = ds_indices.len().max(1);

    // Downsampled, band-limited spectrogram used for median estimation.
    let mut band_ds = vec![vec![0.0f32; band_bins]; n_ds];
    for (k, &t) in ds_indices.iter().enumerate() {
        let src = &magnitude_spec_frames[t][bin_start..bin_end];
        band_ds[k].copy_from_slice(src);
    }

    #[inline]
    fn median_in_place(v: &mut [f32]) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        let mid = v.len() / 2;
        let (_, m, _) = v.select_nth_unstable_by(mid, |a, b| {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        });
        *m
    }

    // Harmonic estimate: median across time (horizontal median filter).
    let mut h_est = vec![vec![0.0f32; band_bins]; n_ds];
    let mut scratch: Vec<f32> = Vec::with_capacity(2 * time_margin + 1);
    for b in 0..band_bins {
        for t in 0..n_ds {
            scratch.clear();
            let start = t.saturating_sub(time_margin);
            let end = (t + time_margin + 1).min(n_ds);
            for tt in start..end {
                let x = band_ds[tt][b];
                scratch.push(if x.is_finite() { x.max(0.0) } else { 0.0 });
            }
            h_est[t][b] = median_in_place(&mut scratch);
        }
    }

    // Percussive estimate: median across frequency (vertical median filter).
    let mut p_est = vec![vec![0.0f32; band_bins]; n_ds];
    scratch = Vec::with_capacity(2 * freq_margin + 1);
    for t in 0..n_ds {
        for b in 0..band_bins {
            scratch.clear();
            let start = b.saturating_sub(freq_margin);
            let end = (b + freq_margin + 1).min(band_bins);
            for bb in start..end {
                let x = band_ds[t][bb];
                scratch.push(if x.is_finite() { x.max(0.0) } else { 0.0 });
            }
            p_est[t][b] = median_in_place(&mut scratch);
        }
    }

    // Harmonic soft mask (computed on downsampled band).
    let p = mask_power.max(1.0);
    let eps = 1e-12f32;
    let mut mask_ds = vec![vec![0.0f32; band_bins]; n_ds];
    for t in 0..n_ds {
        for b in 0..band_bins {
            let h = h_est[t][b].max(0.0);
            let per = p_est[t][b].max(0.0);
            let hp = h.powf(p);
            let pp = per.powf(p);
            mask_ds[t][b] = hp / (hp + pp + eps);
        }
    }

    // Apply mask back to the full-resolution spectrogram.
    let mut out = vec![vec![0.0f32; n_bins]; n_frames];
    for t in 0..n_frames {
        let k = (t / step).min(n_ds - 1);
        for b in 0..band_bins {
            let bin = bin_start + b;
            let x = magnitude_spec_frames[t][bin];
            let x = if x.is_finite() { x.max(0.0) } else { 0.0 };
            out[t][bin] = x * mask_ds[k][b];
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_chroma_empty() {
        let result = extract_chroma(&[], 44100, 2048, 512);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_extract_chroma_short() {
        // Very short audio (less than one frame)
        let samples = vec![0.0f32; 1000];
        let result = extract_chroma(&samples, 44100, 2048, 512);
        assert!(result.is_ok());
        let chroma_vectors = result.unwrap();
        assert_eq!(chroma_vectors.len(), 0);
    }
    
    #[test]
    fn test_extract_chroma_basic() {
        // Generate a simple sine wave at A4 (440 Hz)
        let sample_rate = 44100;
        let duration_samples = sample_rate * 2; // 2 seconds
        let mut samples = Vec::with_capacity(duration_samples);
        
        for i in 0..duration_samples {
            let t = i as f32 / sample_rate as f32;
            samples.push((2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }
        
        let result = extract_chroma(&samples, sample_rate as u32, 2048, 512);
        assert!(result.is_ok());
        
        let chroma_vectors = result.unwrap();
        assert!(!chroma_vectors.is_empty());
        
        // Check that each chroma vector has 12 elements
        for chroma in &chroma_vectors {
            assert_eq!(chroma.len(), 12);
            
            // Check normalization (L2 norm should be ~1.0)
            let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 0.01 || norm < EPSILON);
        }
        
        // A4 should map to semitone class 9 (A)
        // Check that A (index 9) has significant energy
        let avg_chroma: Vec<f32> = (0..12)
            .map(|i| {
                chroma_vectors.iter().map(|v| v[i]).sum::<f32>() / chroma_vectors.len() as f32
            })
            .collect();
        
        // A (index 9) should be prominent
        assert!(avg_chroma[9] > 0.1, "A semitone class should be prominent for A4 tone");
    }
    
    #[test]
    fn test_frame_to_chroma() {
        let sample_rate = 44100;
        let fft_size = 2048;
        
        // Create a magnitude spectrum with energy at A4 (440 Hz)
        let mut magnitude = vec![0.0f32; fft_size / 2 + 1];
        let bin_a4 = (440.0 * fft_size as f32 / sample_rate as f32) as usize;
        if bin_a4 < magnitude.len() {
            magnitude[bin_a4] = 1.0;
        }
        
        let chroma = frame_to_chroma(&magnitude, sample_rate, fft_size, false, 0.5).unwrap();
        assert_eq!(chroma.len(), 12);
        
        // Check normalization
        let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01 || norm < EPSILON);
    }
    
    #[test]
    fn test_invalid_parameters() {
        let samples = vec![0.0f32; 10000];
        
        // Zero frame size
        assert!(extract_chroma(&samples, 44100, 0, 512).is_err());
        
        // Zero hop size
        assert!(extract_chroma(&samples, 44100, 2048, 0).is_err());
        
        // Zero sample rate
        assert!(extract_chroma(&samples, 0, 2048, 512).is_err());
    }
    
    #[test]
    fn test_soft_chroma_mapping() {
        let sample_rate = 44100;
        let duration_samples = sample_rate * 2; // 2 seconds
        let mut samples = Vec::with_capacity(duration_samples);
        
        for i in 0..duration_samples {
            let t = i as f32 / sample_rate as f32;
            samples.push((2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }
        
        // Test with soft mapping enabled
        let result_soft = extract_chroma_with_options(&samples, sample_rate as u32, 2048, 512, true, 0.5);
        assert!(result_soft.is_ok());
        
        // Test with soft mapping disabled
        let result_hard = extract_chroma_with_options(&samples, sample_rate as u32, 2048, 512, false, 0.5);
        assert!(result_hard.is_ok());
        
        // Both should produce valid chroma vectors
        let chroma_soft = result_soft.unwrap();
        let chroma_hard = result_hard.unwrap();
        
        assert_eq!(chroma_soft.len(), chroma_hard.len());
        assert!(!chroma_soft.is_empty());
    }
}
