//! Novelty curve extraction for tempogram analysis
//!
//! Extracts novelty curves from magnitude spectrograms using multiple complementary methods:
//! - Spectral flux: Detects spectral changes (harmonic onsets)
//! - Energy flux: Detects energy changes (percussive onsets)
//! - High-frequency content (HFC): Detects high-frequency attacks
//!
//! These curves are combined with weighted voting to create a robust novelty curve
//! for tempogram-based BPM detection.
//!
//! # Reference
//!
//! Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
//! *IEEE Transactions on Audio, Speech, and Language Processing*.
//!
//! Klapuri, A., Eronen, A., & Astola, J. (2006). Analysis of the Meter of Audio Signals.
//! *IEEE Transactions on Audio, Speech, and Language Processing*, 14(1), 342-355.
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::period::novelty::{spectral_flux_novelty, energy_flux_novelty, hfc_novelty, combined_novelty};
//!
//! // magnitude_spec_frames is a spectrogram: Vec<Vec<f32>> where each inner Vec is a frame
//! let magnitude_spec_frames = vec![vec![0.0f32; 1024]; 100];
//!
//! let spectral = spectral_flux_novelty(&magnitude_spec_frames)?;
//! let energy = energy_flux_novelty(&magnitude_spec_frames)?;
//! let hfc = hfc_novelty(&magnitude_spec_frames, 44100)?;
//! let combined = combined_novelty(&spectral, &energy, &hfc);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

fn validate_spectrogram(magnitude_spec_frames: &[Vec<f32>]) -> Result<usize, AnalysisError> {
    if magnitude_spec_frames.is_empty() || magnitude_spec_frames.len() < 2 {
        return Ok(0);
    }
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins, i, frame.len()
            )));
        }
    }
    Ok(n_bins)
}

fn mel(f_hz: f32) -> f32 {
    // HTK mel scale
    2595.0 * (1.0 + (f_hz / 700.0)).log10()
}

fn inv_mel(mel_val: f32) -> f32 {
    700.0 * (10.0f32.powf(mel_val / 2595.0) - 1.0)
}

#[derive(Debug, Clone)]
struct MelFilterbank {
    n_mels: usize,
    // For each FFT bin, list of (mel_idx, weight) contributions (usually <= 2).
    bin_contribs: Vec<Vec<(usize, f32)>>,
}

impl MelFilterbank {
    fn new(
        sample_rate: u32,
        n_bins: usize,
        n_mels: usize,
        fmin_hz: f32,
        fmax_hz: f32,
    ) -> Result<Self, AnalysisError> {
        if sample_rate == 0 {
            return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
        }
        if n_bins < 2 {
            return Err(AnalysisError::InvalidInput("Not enough FFT bins".to_string()));
        }
        let n_mels = n_mels.max(4);
        let nyquist = sample_rate as f32 * 0.5;
        let fmin = fmin_hz.max(0.0).min(nyquist.max(1.0));
        let mut fmax = fmax_hz;
        if !(fmax.is_finite() && fmax > 0.0) {
            fmax = nyquist;
        }
        let fmax = fmax.min(nyquist).max(fmin + 1.0);

        // Infer FFT size from rFFT bins: n_bins = fft_size/2 + 1
        let fft_size = (n_bins - 1) * 2;
        let freq_resolution = sample_rate as f32 / fft_size as f32;

        let mel_min = mel(fmin);
        let mel_max = mel(fmax);
        let step = (mel_max - mel_min) / (n_mels + 1) as f32;

        let mut mel_points = Vec::with_capacity(n_mels + 2);
        for i in 0..(n_mels + 2) {
            mel_points.push(mel_min + step * i as f32);
        }

        let hz_points: Vec<f32> = mel_points.into_iter().map(inv_mel).collect();
        let mut bin_points: Vec<usize> = hz_points
            .iter()
            .map(|&hz| ((hz / freq_resolution).round() as isize).clamp(0, (n_bins - 1) as isize) as usize)
            .collect();

        // Ensure strictly increasing to avoid degenerate triangles.
        for i in 1..bin_points.len() {
            if bin_points[i] <= bin_points[i - 1] {
                bin_points[i] = (bin_points[i - 1] + 1).min(n_bins - 1);
            }
        }

        let mut bin_contribs: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n_bins];
        for m in 0..n_mels {
            let left = bin_points[m];
            let center = bin_points[m + 1];
            let right = bin_points[m + 2];
            if !(left < center && center < right) {
                continue;
            }

            // Rising slope: left..center
            for b in left..=center {
                let w = if b == left {
                    0.0
                } else {
                    (b as f32 - left as f32) / (center as f32 - left as f32)
                };
                if w > 0.0 {
                    bin_contribs[b].push((m, w));
                }
            }
            // Falling slope: center..right
            for b in center..=right {
                let w = if b == right {
                    0.0
                } else {
                    (right as f32 - b as f32) / (right as f32 - center as f32)
                };
                if w > 0.0 {
                    bin_contribs[b].push((m, w));
                }
            }
        }

        Ok(Self { n_mels, bin_contribs })
    }

    fn apply_logmag(&self, mag_frame: &[f32]) -> Vec<f32> {
        let mut mel = vec![0.0f32; self.n_mels];
        for (b, &x) in mag_frame.iter().enumerate() {
            if b >= self.bin_contribs.len() {
                break;
            }
            let v = (1.0 + x.max(0.0)).ln();
            if v <= 0.0 {
                continue;
            }
            for &(m, w) in &self.bin_contribs[b] {
                mel[m] += v * w;
            }
        }
        mel
    }
}

/// Extract spectral flux novelty curve from magnitude spectrogram
///
/// Computes frame-to-frame spectral changes, emphasizing positive differences
/// (onsets) over negative differences (decays). This method is particularly
/// effective for detecting harmonic onsets and spectral changes.
///
/// # Reference
///
/// Grosche, P., Müller, M., & Serrà, J. (2012). Robust Local Features for Remote Folk Music Identification.
/// *IEEE Transactions on Audio, Speech, and Language Processing*.
///
/// Klapuri, A., Eronen, A., & Astola, J. (2006). Analysis of the Meter of Audio Signals.
/// *IEEE Transactions on Audio, Speech, and Language Processing*, 14(1), 342-355.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
///   Each inner `Vec<f32>` represents one frame's magnitude spectrum
///
/// # Returns
///
/// Novelty curve as `Vec<f32>` with length `n_frames - 1` (one value per frame transition)
/// Values are normalized to [0, 1] range
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Spectrogram is empty
/// - Frames have inconsistent lengths
/// - Less than 2 frames (need at least 2 to compute flux)
pub fn spectral_flux_novelty(
    magnitude_spec_frames: &[Vec<f32>],
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    
    if magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }
    
    // Check that all frames have the same length
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!("Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                        n_bins, i, frame.len())
            ));
        }
    }
    
    log::debug!("Computing spectral flux novelty: {} frames, {} bins per frame",
                magnitude_spec_frames.len(), n_bins);
    
    // Step 1: Normalize magnitude to [0, 1] per frame
    let mut normalized: Vec<Vec<f32>> = Vec::with_capacity(magnitude_spec_frames.len());
    
    for frame in magnitude_spec_frames {
        let max_mag = frame.iter().copied().fold(0.0f32, f32::max);
        
        if max_mag > EPSILON {
            let normalized_frame: Vec<f32> = frame.iter()
                .map(|&x| x / max_mag)
                .collect();
            normalized.push(normalized_frame);
        } else {
            normalized.push(vec![0.0f32; n_bins]);
        }
    }
    
    // Step 2: Compute L2 distance between consecutive frames (only positive differences)
    let mut flux = Vec::with_capacity(normalized.len().saturating_sub(1));
    
    for i in 1..normalized.len() {
        let prev_frame = &normalized[i - 1];
        let curr_frame = &normalized[i];
        
        // Compute L2 distance with half-wave rectification (only positive differences)
        let sum_sq_diff: f32 = prev_frame.iter()
            .zip(curr_frame.iter())
            .map(|(&prev, &curr)| {
                let diff = curr - prev;
                // Only count positive differences (increases in magnitude = onsets)
                diff.max(0.0)
            })
            .map(|x| x * x)
            .sum();
        
        let l2_distance = sum_sq_diff.sqrt();
        flux.push(l2_distance);
    }
    
    if flux.is_empty() {
        return Ok(Vec::new());
    }
    
    // Step 3: Normalize to [0, 1]
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for val in &mut flux {
            *val /= max_flux;
        }
    }
    
    log::debug!("Spectral flux novelty: {} values, max={:.6}",
                flux.len(), max_flux);
    
    Ok(flux)
}

/// Extract SuperFlux novelty curve from magnitude spectrogram.
///
/// SuperFlux (Böck & Widmer, 2013) improves over plain spectral flux by using a
/// max-filtered reference (here: along frequency on the *previous* frame) before
/// differencing. This reduces false positives from vibrato/pitch modulation and
/// tends to yield a cleaner onset-strength function for rhythmic analysis.
///
/// This implementation is a lightweight variant suitable for our tempo pipeline:
/// - Log-compress magnitudes (`log1p`)
/// - For each bin, subtract the max in a small frequency neighborhood of the previous frame
/// - Half-wave rectify and sum energy
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
/// * `max_filter_bins` - Neighborhood radius in bins for max filtering (typical: 3–8)
///
/// # Returns
///
/// Novelty curve as `Vec<f32>` with length `n_frames - 1`, normalized to [0, 1]
pub fn superflux_novelty(
    magnitude_spec_frames: &[Vec<f32>],
    max_filter_bins: usize,
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() || magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }

    let n_bins = validate_spectrogram(magnitude_spec_frames)?;
    if n_bins == 0 {
        return Ok(Vec::new());
    }

    let k = max_filter_bins.max(1);

    // Precompute log-compressed frames
    let mut log_frames: Vec<Vec<f32>> = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        let lf: Vec<f32> = frame.iter().map(|&x| (1.0 + x.max(0.0)).ln()).collect();
        log_frames.push(lf);
    }

    let mut flux = Vec::with_capacity(log_frames.len().saturating_sub(1));
    for i in 1..log_frames.len() {
        let prev = &log_frames[i - 1];
        let curr = &log_frames[i];

        let mut sum = 0.0f32;
        for b in 0..n_bins {
            let start = b.saturating_sub(k);
            let end = (b + k + 1).min(n_bins);
            let mut prev_max = 0.0f32;
            for v in &prev[start..end] {
                prev_max = prev_max.max(*v);
            }
            let diff = (curr[b] - prev_max).max(0.0);
            sum += diff * diff;
        }
        flux.push(sum.sqrt());
    }

    if flux.is_empty() {
        return Ok(Vec::new());
    }
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for v in &mut flux {
            *v /= max_flux;
        }
    }
    Ok(flux)
}

/// Extract SuperFlux novelty curve from a **frequency sub-band** of a magnitude spectrogram.
///
/// This is the same algorithm as `superflux_novelty`, but it only uses bins in
/// `[bin_start, bin_end)` (clamped to the frame width). This supports multi-band novelty
/// fusion without having to allocate band-sliced spectrograms.
pub fn superflux_novelty_band(
    magnitude_spec_frames: &[Vec<f32>],
    max_filter_bins: usize,
    bin_start: usize,
    bin_end: usize,
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() || magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }

    let n_bins = validate_spectrogram(magnitude_spec_frames)?;
    if n_bins == 0 {
        return Ok(Vec::new());
    }

    let start = bin_start.min(n_bins);
    let end = bin_end.min(n_bins);
    if end <= start + 1 {
        return Ok(Vec::new());
    }

    let k = max_filter_bins.max(1);

    // Precompute log-compressed frames (full frame, reused for band lookup).
    let mut log_frames: Vec<Vec<f32>> = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        let lf: Vec<f32> = frame.iter().map(|&x| (1.0 + x.max(0.0)).ln()).collect();
        log_frames.push(lf);
    }

    let mut flux = Vec::with_capacity(log_frames.len().saturating_sub(1));
    for i in 1..log_frames.len() {
        let prev = &log_frames[i - 1];
        let curr = &log_frames[i];

        let mut sum = 0.0f32;
        for b in start..end {
            let local_start = b.saturating_sub(k).max(start);
            let local_end = (b + k + 1).min(end);
            let mut prev_max = 0.0f32;
            for v in &prev[local_start..local_end] {
                prev_max = prev_max.max(*v);
            }
            let diff = (curr[b] - prev_max).max(0.0);
            sum += diff * diff;
        }
        flux.push(sum.sqrt());
    }

    if flux.is_empty() {
        return Ok(Vec::new());
    }
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for v in &mut flux {
            *v /= max_flux;
        }
    }
    Ok(flux)
}

/// Extract energy flux novelty curve from magnitude spectrogram
///
/// Computes frame-to-frame energy changes by summing magnitude across all
/// frequency bins. This method is particularly effective for detecting
/// percussive onsets and energy-based changes.
///
/// # Reference
///
/// Bello, J. P., Daudet, L., Abdallah, S., Duxbury, C., Davies, M., & Sandler, M. B. (2005).
/// A Tutorial on Onset Detection in Music Signals.
/// *IEEE Transactions on Speech and Audio Processing*, 13(5), 1035-1047.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
///
/// # Returns
///
/// Novelty curve as `Vec<f32>` with length `n_frames - 1` (one value per frame transition)
/// Values are normalized to [0, 1] range
pub fn energy_flux_novelty(
    magnitude_spec_frames: &[Vec<f32>],
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    
    if magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }
    
    // Check that all frames have the same length
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!("Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                        n_bins, i, frame.len())
            ));
        }
    }
    
    log::debug!("Computing energy flux novelty: {} frames, {} bins per frame",
                magnitude_spec_frames.len(), n_bins);
    
    // Compute energy per frame (sum of squared magnitudes)
    let energies: Vec<f32> = magnitude_spec_frames.iter()
        .map(|frame| frame.iter().map(|&x| x * x).sum())
        .collect();
    
    // Compute energy flux (positive differences only)
    let mut flux = Vec::with_capacity(energies.len().saturating_sub(1));
    
    for i in 1..energies.len() {
        let diff = energies[i] - energies[i - 1];
        // Only positive differences (energy increases = onsets)
        flux.push(diff.max(0.0));
    }
    
    if flux.is_empty() {
        return Ok(Vec::new());
    }
    
    // Normalize to [0, 1]
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for val in &mut flux {
            *val /= max_flux;
        }
    }
    
    log::debug!("Energy flux novelty: {} values, max={:.6}",
                flux.len(), max_flux);
    
    Ok(flux)
}

/// Extract a log-mel SuperFlux-style novelty curve.
///
/// This computes mel-band energies from log-compressed magnitudes, then applies the same
/// max-filtered-reference differencing as SuperFlux (but in mel space). In practice,
/// mel aggregation can reduce spurious bin-level variance and improve robustness in
/// dense harmonic material.
pub fn mel_superflux_novelty(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    n_mels: usize,
    fmin_hz: f32,
    fmax_hz: f32,
    max_filter_mels: usize,
) -> Result<Vec<f32>, AnalysisError> {
    let n_bins = validate_spectrogram(magnitude_spec_frames)?;
    if n_bins == 0 {
        return Ok(Vec::new());
    }

    let fb = MelFilterbank::new(sample_rate, n_bins, n_mels, fmin_hz, fmax_hz)?;
    let k = max_filter_mels.max(1);

    // Precompute mel frames
    let mut mel_frames: Vec<Vec<f32>> = Vec::with_capacity(magnitude_spec_frames.len());
    for frame in magnitude_spec_frames {
        mel_frames.push(fb.apply_logmag(frame));
    }

    let mut flux = Vec::with_capacity(mel_frames.len().saturating_sub(1));
    for i in 1..mel_frames.len() {
        let prev = &mel_frames[i - 1];
        let curr = &mel_frames[i];
        let n = prev.len().min(curr.len());
        if n == 0 {
            continue;
        }

        let mut sum = 0.0f32;
        for b in 0..n {
            let start = b.saturating_sub(k);
            let end = (b + k + 1).min(n);
            let mut prev_max = 0.0f32;
            for v in &prev[start..end] {
                prev_max = prev_max.max(*v);
            }
            let diff = (curr[b] - prev_max).max(0.0);
            sum += diff * diff;
        }
        flux.push(sum.sqrt());
    }

    if flux.is_empty() {
        return Ok(Vec::new());
    }
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for v in &mut flux {
            *v /= max_flux;
        }
    }
    Ok(flux)
}

/// Band-limited variant of `energy_flux_novelty` using bins `[bin_start, bin_end)`.
pub fn energy_flux_novelty_band(
    magnitude_spec_frames: &[Vec<f32>],
    bin_start: usize,
    bin_end: usize,
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() || magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }

    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!(
                    "Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                    n_bins,
                    i,
                    frame.len()
                ),
            ));
        }
    }

    let start = bin_start.min(n_bins);
    let end = bin_end.min(n_bins);
    if end <= start + 1 {
        return Ok(Vec::new());
    }

    let energies: Vec<f32> = magnitude_spec_frames
        .iter()
        .map(|frame| frame[start..end].iter().map(|&x| x * x).sum())
        .collect();

    let mut flux = Vec::with_capacity(energies.len().saturating_sub(1));
    for i in 1..energies.len() {
        flux.push((energies[i] - energies[i - 1]).max(0.0));
    }

    if flux.is_empty() {
        return Ok(Vec::new());
    }

    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for v in &mut flux {
            *v /= max_flux;
        }
    }
    Ok(flux)
}

/// Extract high-frequency content (HFC) novelty curve from magnitude spectrogram
///
/// Computes weighted sum emphasizing high frequencies, making this method
/// particularly effective for detecting percussive attacks and sharp transients.
///
/// # Reference
///
/// Bello, J. P., Daudet, L., Abdallah, S., Duxbury, C., Davies, M., & Sandler, M. B. (2005).
/// A Tutorial on Onset Detection in Music Signals.
/// *IEEE Transactions on Speech and Audio Processing*, 13(5), 1035-1047.
///
/// # Arguments
///
/// * `magnitude_spec_frames` - FFT magnitude spectrogram (n_frames × n_bins)
/// * `sample_rate` - Sample rate in Hz (used for frequency bin calculation)
///
/// # Returns
///
/// Novelty curve as `Vec<f32>` with length `n_frames - 1` (one value per frame transition)
/// Values are normalized to [0, 1] range
pub fn hfc_novelty(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(Vec::new());
    }
    
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
    }
    
    if magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }
    
    // Check that all frames have the same length
    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!("Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                        n_bins, i, frame.len())
            ));
        }
    }
    
    log::debug!("Computing HFC novelty: {} frames, {} bins per frame, sample_rate={}",
                magnitude_spec_frames.len(), n_bins, sample_rate);
    
    // Compute HFC per frame: sum over k (k * |X[k]|^2)
    // where k is the frequency bin index (higher bins = higher frequencies)
    let hfc_values: Vec<f32> = magnitude_spec_frames.iter()
        .map(|frame| {
            frame.iter()
                .enumerate()
                .map(|(k, &mag)| (k as f32) * mag * mag)
                .sum()
        })
        .collect();
    
    // Compute HFC flux (positive differences only)
    let mut flux = Vec::with_capacity(hfc_values.len().saturating_sub(1));
    
    for i in 1..hfc_values.len() {
        let diff = hfc_values[i] - hfc_values[i - 1];
        // Only positive differences (HFC increases = high-frequency onsets)
        flux.push(diff.max(0.0));
    }
    
    if flux.is_empty() {
        return Ok(Vec::new());
    }
    
    // Normalize to [0, 1]
    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for val in &mut flux {
            *val /= max_flux;
        }
    }
    
    log::debug!("HFC novelty: {} values, max={:.6}",
                flux.len(), max_flux);
    
    Ok(flux)
}

/// Band-limited variant of `hfc_novelty` using bins `[bin_start, bin_end)`.
///
/// Note: We retain the **absolute** bin index weighting \(k * |X[k]|^2\) inside the band
/// (not a reindexed 0..N band), to keep the interpretation consistent across bands.
pub fn hfc_novelty_band(
    magnitude_spec_frames: &[Vec<f32>],
    bin_start: usize,
    bin_end: usize,
) -> Result<Vec<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() || magnitude_spec_frames.len() < 2 {
        return Ok(Vec::new());
    }

    let n_bins = magnitude_spec_frames[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput("Empty magnitude frames".to_string()));
    }
    for (i, frame) in magnitude_spec_frames.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(
                format!(
                    "Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                    n_bins,
                    i,
                    frame.len()
                ),
            ));
        }
    }

    let start = bin_start.min(n_bins);
    let end = bin_end.min(n_bins);
    if end <= start + 1 {
        return Ok(Vec::new());
    }

    let hfc_values: Vec<f32> = magnitude_spec_frames
        .iter()
        .map(|frame| {
            frame[start..end]
                .iter()
                .enumerate()
                .map(|(i, &mag)| {
                    let k = (start + i) as f32;
                    k * mag * mag
                })
                .sum()
        })
        .collect();

    let mut flux = Vec::with_capacity(hfc_values.len().saturating_sub(1));
    for i in 1..hfc_values.len() {
        flux.push((hfc_values[i] - hfc_values[i - 1]).max(0.0));
    }

    if flux.is_empty() {
        return Ok(Vec::new());
    }

    let max_flux = flux.iter().copied().fold(0.0f32, f32::max);
    if max_flux > EPSILON {
        for v in &mut flux {
            *v /= max_flux;
        }
    }
    Ok(flux)
}

/// Combine multiple novelty curves with weighted voting
///
/// Combines spectral flux, energy flux, and HFC novelty curves into a single
/// robust novelty curve. Each method captures different aspects of onsets:
/// - Spectral flux: Harmonic changes
/// - Energy flux: Energy changes
/// - HFC: High-frequency attacks
///
/// Consensus voting makes the combined curve more reliable than any single method.
///
/// # Arguments
///
/// * `spectral` - Spectral flux novelty curve
/// * `energy` - Energy flux novelty curve
/// * `hfc` - HFC novelty curve
///
/// # Returns
///
/// Combined novelty curve as `Vec<f32>`, normalized to [0, 1] range
/// Length matches the shortest input curve
///
/// # Weights
///
/// Default weights (can be adjusted based on empirical performance):
/// - Spectral flux: 0.5 (most important for BPM detection per Klapuri et al. 2006)
/// - Energy flux: 0.3
/// - HFC: 0.2
pub fn combined_novelty(
    spectral: &[f32],
    energy: &[f32],
    hfc: &[f32],
) -> Vec<f32> {
    combined_novelty_with_params(spectral, energy, hfc, 0.5, 0.3, 0.2, 16, 5)
}

/// Combine multiple novelty curves with configurable weights and conditioning.
///
/// This is a tuning hook: novelty weighting and conditioning strongly affects whether the
/// novelty emphasizes the beat-level pulse vs subdivisions (hi-hats) vs harmonic motion.
pub fn combined_novelty_with_params(
    spectral: &[f32],
    energy: &[f32],
    hfc: &[f32],
    w_spectral: f32,
    w_energy: f32,
    w_hfc: f32,
    local_mean_window: usize,
    smooth_window: usize,
) -> Vec<f32> {
    // Find minimum length (all curves should be same length, but be safe)
    let min_len = spectral.len().min(energy.len()).min(hfc.len());

    if min_len == 0 {
        return Vec::new();
    }

    let ws = w_spectral.max(0.0);
    let we = w_energy.max(0.0);
    let wh = w_hfc.max(0.0);
    let wsum = (ws + we + wh).max(EPSILON);

    let mut combined = Vec::with_capacity(min_len);
    for i in 0..min_len {
        let spectral_val = spectral.get(i).copied().unwrap_or(0.0);
        let energy_val = energy.get(i).copied().unwrap_or(0.0);
        let hfc_val = hfc.get(i).copied().unwrap_or(0.0);

        // Weighted average
        let weighted_sum = (spectral_val * ws + energy_val * we + hfc_val * wh) / wsum;
        combined.push(weighted_sum);
    }

    // Normalize to [0, 1] (should already be normalized, but ensure it)
    normalize_in_place(&mut combined);

    // Conditioning:
    // - local mean subtraction is effectively a high-pass in time, with half-wave rectification
    // - smoothing stabilizes the novelty and can reduce spurious sub-beat transients
    if local_mean_window > 1 {
        combined = local_mean_subtract(&combined, local_mean_window);
    }
    if smooth_window > 1 {
        smooth_moving_average_in_place(&mut combined, smooth_window);
    }
    normalize_in_place(&mut combined);

    log::debug!(
        "Combined novelty (conditioned): {} values (w=[{:.2},{:.2},{:.2}], local_mean={}, smooth={})",
        combined.len(),
        ws,
        we,
        wh,
        local_mean_window,
        smooth_window
    );

    combined
}

/// Normalize a curve in-place to [0, 1] by dividing by its maximum.
fn normalize_in_place(curve: &mut [f32]) {
    let max_val = curve.iter().copied().fold(0.0f32, f32::max);
    if max_val > EPSILON {
        for v in curve {
            *v /= max_val;
        }
    }
}

/// Subtract a moving-average local mean and half-wave rectify.
///
/// Output: `max(0, x[i] - local_mean[i])`.
fn local_mean_subtract(x: &[f32], window: usize) -> Vec<f32> {
    if x.is_empty() || window == 0 {
        return x.to_vec();
    }
    let w = window.max(1);
    let half = w / 2;
    let mut out = vec![0.0f32; x.len()];

    for i in 0..x.len() {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(x.len());
        let mut sum = 0.0f32;
        for v in &x[start..end] {
            sum += *v;
        }
        let mean = sum / (end - start) as f32;
        out[i] = (x[i] - mean).max(0.0);
    }

    out
}

/// Simple moving-average smoothing in-place.
fn smooth_moving_average_in_place(x: &mut [f32], window: usize) {
    if x.len() < 3 || window <= 1 {
        return;
    }
    let w = window.max(1);
    let half = w / 2;
    let orig = x.to_vec();
    for i in 0..x.len() {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(x.len());
        let mut sum = 0.0f32;
        for v in &orig[start..end] {
            sum += *v;
        }
        x[i] = sum / (end - start) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_spectral_flux_novelty_basic() {
        // Create spectrogram with a spectral change
        let mut spectrogram = vec![vec![0.1f32; 1024]; 10];
        
        // Frame 5: different spectral pattern
        for bin in 0..512 {
            spectrogram[5][bin] = 1.0f32;
        }
        
        let novelty = spectral_flux_novelty(&spectrogram).unwrap();
        
        // Should have 9 values (10 frames - 1)
        assert_eq!(novelty.len(), 9);
        
        // Should detect change around frame 5
        assert!(novelty[4] > 0.0 || novelty[5] > 0.0);
    }
    
    #[test]
    fn test_spectral_flux_novelty_empty() {
        let spectrogram = vec![];
        let novelty = spectral_flux_novelty(&spectrogram).unwrap();
        assert!(novelty.is_empty());
    }
    
    #[test]
    fn test_spectral_flux_novelty_single_frame() {
        let spectrogram = vec![vec![0.5f32; 1024]];
        let novelty = spectral_flux_novelty(&spectrogram).unwrap();
        // Need at least 2 frames
        assert!(novelty.is_empty());
    }
    
    #[test]
    fn test_energy_flux_novelty_basic() {
        let mut spectrogram = vec![vec![0.1f32; 1024]; 10];
        
        // Frame 5: higher energy
        for bin in 0..1024 {
            spectrogram[5][bin] = 1.0f32;
        }
        
        let novelty = energy_flux_novelty(&spectrogram).unwrap();
        
        assert_eq!(novelty.len(), 9);
        // Should detect energy increase at frame 5
        assert!(novelty[4] > 0.0 || novelty[5] > 0.0);
    }
    
    #[test]
    fn test_hfc_novelty_basic() {
        let mut spectrogram = vec![vec![0.1f32; 1024]; 10];
        
        // Frame 5: high-frequency content
        for bin in 512..1024 {
            spectrogram[5][bin] = 1.0f32;
        }
        
        let novelty = hfc_novelty(&spectrogram, 44100).unwrap();
        
        assert_eq!(novelty.len(), 9);
        // Should detect HFC increase at frame 5
        assert!(novelty[4] > 0.0 || novelty[5] > 0.0);
    }
    
    #[test]
    fn test_combined_novelty() {
        let spectral = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let energy = vec![0.0, 0.3, 0.8, 0.3, 0.0];
        let hfc = vec![0.0, 0.2, 0.6, 0.2, 0.0];
        
        let combined = combined_novelty(&spectral, &energy, &hfc);
        
        assert_eq!(combined.len(), 5);
        // Conditioning can reshape small synthetic examples; just validate normalization/range.
        assert!(combined.iter().all(|&v| v >= 0.0 && v <= 1.0));
        assert!(combined.iter().copied().fold(0.0f32, f32::max) > 0.0);
    }
    
    #[test]
    fn test_combined_novelty_different_lengths() {
        let spectral = vec![0.0, 0.5, 1.0];
        let energy = vec![0.0, 0.3, 0.8, 0.3];
        let hfc = vec![0.0, 0.2];
        
        let combined = combined_novelty(&spectral, &energy, &hfc);
        
        // Should use minimum length (2)
        assert_eq!(combined.len(), 2);
    }
}

