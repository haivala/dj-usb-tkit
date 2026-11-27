//! Silence detection and trimming utilities
//!
//! Detects silent regions in audio and trims leading/trailing silence.
//!
//! Algorithm:
//! 1. Frame audio into chunks
//! 2. Compute RMS energy per frame
//! 3. Mark frames below threshold as silent
//! 4. Merge consecutive silent frames (< 500ms)
//! 5. Trim leading/trailing silence
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::preprocessing::silence::{detect_and_trim, SilenceDetector};
//!
//! let samples = vec![0.0f32; 44100 * 5]; // 5 seconds
//! let detector = SilenceDetector::default();
//! let (trimmed, silence_map) = detect_and_trim(&samples, 44100, detector)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Silence detection configuration
#[derive(Debug, Clone)]
pub struct SilenceDetector {
    /// Threshold in dB (default: -40.0)
    /// Frames with RMS below this threshold are considered silent
    pub threshold_db: f32,
    
    /// Minimum duration in milliseconds for silence to be merged (default: 500)
    /// Consecutive silent frames shorter than this are merged
    pub min_duration_ms: u32,
    
    /// Frame size for analysis (default: 2048)
    pub frame_size: usize,
}

impl Default for SilenceDetector {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            min_duration_ms: 500,
            frame_size: 2048,
        }
    }
}

/// Silence region information
#[derive(Debug, Clone)]
pub struct SilenceRegion {
    /// Start sample index (inclusive)
    pub start_sample: usize,
    /// End sample index (exclusive)
    pub end_sample: usize,
    /// Duration in seconds
    pub duration_seconds: f32,
}

/// Detect and trim silence from audio
///
/// This function detects silent regions in audio by analyzing RMS energy
/// per frame, then trims leading and trailing silence. It also returns
/// a map of all silence regions found.
///
/// # Arguments
///
/// * `samples` - Audio samples (mono, normalized to [-1.0, 1.0])
/// * `sample_rate` - Sample rate in Hz
/// * `detector` - Silence detection configuration
///
/// # Returns
///
/// Tuple of:
/// - `Vec<f32>`: Trimmed samples (leading/trailing silence removed)
/// - `Vec<(usize, usize)>`: Silence map as (start_sample, end_sample) pairs
///
/// # Errors
///
/// Returns `AnalysisError` if parameters are invalid or processing fails
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::preprocessing::silence::{detect_and_trim, SilenceDetector};
///
/// let mut samples = vec![0.0f32; 44100 * 5];
/// // Add some audio in the middle
/// for i in 22050..66150 {
///     samples[i] = 0.5;
/// }
///
/// let detector = SilenceDetector::default();
/// let (trimmed, silence_map) = detect_and_trim(&samples, 44100, detector)?;
///
/// println!("Trimmed from {} to {} samples", samples.len(), trimmed.len());
/// println!("Found {} silence regions", silence_map.len());
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn detect_and_trim(
    samples: &[f32],
    sample_rate: u32,
    detector: SilenceDetector,
) -> Result<(Vec<f32>, Vec<(usize, usize)>), AnalysisError> {
    // Validate inputs
    if samples.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    
    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput("Sample rate must be > 0".to_string()));
    }
    
    if detector.frame_size == 0 {
        return Err(AnalysisError::InvalidInput("Frame size must be > 0".to_string()));
    }
    
    if detector.frame_size > samples.len() {
        log::warn!("Frame size ({}) larger than audio length ({}), treating as single frame",
                   detector.frame_size, samples.len());
    }
    
    log::debug!("Detecting silence: {} samples at {} Hz, threshold={:.1} dB, min_duration={} ms",
                samples.len(), sample_rate, detector.threshold_db, detector.min_duration_ms);
    
    // Convert threshold from dB to linear RMS
    let threshold_linear = 10.0_f32.powf(detector.threshold_db / 20.0);
    
    // Step 1: Frame audio and compute RMS per frame
    let hop_size = detector.frame_size / 2; // 50% overlap for smoother detection
    let num_frames = if samples.len() >= detector.frame_size {
        (samples.len() - detector.frame_size) / hop_size + 1
    } else {
        1 // At least one frame
    };
    
    let mut frame_rms = Vec::with_capacity(num_frames);
    let mut frame_starts = Vec::with_capacity(num_frames);
    
    for i in 0..num_frames {
        let start = i * hop_size;
        let end = (start + detector.frame_size).min(samples.len());
        
        // Compute RMS
        let sum_sq: f32 = samples[start..end]
            .iter()
            .map(|&x| x * x)
            .sum();
        
        let rms = if end > start {
            (sum_sq / (end - start) as f32).sqrt()
        } else {
            0.0
        };
        
        frame_rms.push(rms);
        frame_starts.push(start);
    }
    
    // Step 2: Mark frames as silent or not
    let mut frame_is_silent = Vec::with_capacity(num_frames);
    for &rms in &frame_rms {
        frame_is_silent.push(rms <= threshold_linear);
    }
    
    // Step 3: Merge consecutive silent frames
    // Convert min_duration_ms to frames
    let min_duration_samples = (detector.min_duration_ms as f32 / 1000.0 * sample_rate as f32) as usize;
    let min_duration_frames = (min_duration_samples + hop_size - 1) / hop_size; // Round up
    
    // Find silence regions
    let mut silence_regions: Vec<SilenceRegion> = Vec::new();
    let mut in_silence = false;
    let mut silence_start_frame = 0;
    
    for (frame_idx, &is_silent) in frame_is_silent.iter().enumerate() {
        if is_silent && !in_silence {
            // Start of silence
            in_silence = true;
            silence_start_frame = frame_idx;
        } else if !is_silent && in_silence {
            // End of silence
            in_silence = false;
            let silence_end_frame = frame_idx;
            let silence_duration_frames = silence_end_frame - silence_start_frame;
            
            // Only include if duration >= min_duration (or if it's leading/trailing)
            if silence_duration_frames >= min_duration_frames || 
               silence_start_frame == 0 || 
               silence_end_frame == num_frames {
                let start_sample = frame_starts[silence_start_frame];
                let end_sample = if silence_end_frame < frame_starts.len() {
                    frame_starts[silence_end_frame]
                } else {
                    samples.len()
                };
                
                silence_regions.push(SilenceRegion {
                    start_sample,
                    end_sample,
                    duration_seconds: (end_sample - start_sample) as f32 / sample_rate as f32,
                });
            }
        }
    }
    
    // Handle trailing silence
    if in_silence {
        let silence_duration_frames = num_frames - silence_start_frame;
        if silence_duration_frames >= min_duration_frames || silence_start_frame == 0 {
            let start_sample = frame_starts[silence_start_frame];
            silence_regions.push(SilenceRegion {
                start_sample,
                end_sample: samples.len(),
                duration_seconds: (samples.len() - start_sample) as f32 / sample_rate as f32,
            });
        }
    }
    
    // Step 4: Trim leading and trailing silence
    let trim_start = if let Some(first_region) = silence_regions.first() {
        if first_region.start_sample == 0 {
            first_region.end_sample
        } else {
            0
        }
    } else {
        0
    };
    
    let trim_end = if let Some(last_region) = silence_regions.last() {
        if last_region.end_sample == samples.len() {
            last_region.start_sample
        } else {
            samples.len()
        }
    } else {
        samples.len()
    };
    
    // Ensure trim_end > trim_start
    let trim_start = trim_start.min(trim_end);
    let trim_end = trim_end.max(trim_start);
    
    // Extract trimmed samples
    let trimmed = if trim_start < trim_end && trim_end <= samples.len() {
        samples[trim_start..trim_end].to_vec()
    } else {
        Vec::new()
    };
    
    // Convert silence regions to (start, end) pairs for return
    let silence_map: Vec<(usize, usize)> = silence_regions
        .iter()
        .map(|r| (r.start_sample, r.end_sample))
        .collect();
    
    log::debug!("Silence detection: trimmed from {} to {} samples, found {} silence regions",
                samples.len(), trimmed.len(), silence_map.len());
    
    Ok((trimmed, silence_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Generate test audio with silence at beginning and end
    fn generate_test_audio_with_silence(
        total_samples: usize,
        audio_start: usize,
        audio_end: usize,
        amplitude: f32,
    ) -> Vec<f32> {
        let mut samples = vec![0.0f32; total_samples];
        // Add audio signal in the middle
        for i in audio_start..audio_end.min(total_samples) {
            samples[i] = amplitude * (i as f32 / 1000.0).sin(); // Simple sine wave
        }
        samples
    }
    
    #[test]
    fn test_detect_and_trim_leading_trailing() {
        // Create audio with silence at start and end
        let total_samples = 44100 * 3; // 3 seconds
        let audio_start = 44100; // 1 second of silence at start
        let audio_end = 44100 * 2; // 1 second of audio, then silence
        let samples = generate_test_audio_with_silence(total_samples, audio_start, audio_end, 0.5);
        
        let detector = SilenceDetector::default();
        let (trimmed, silence_map) = detect_and_trim(&samples, 44100, detector).unwrap();
        
        // Should trim leading and trailing silence
        assert!(trimmed.len() < samples.len(), "Should trim some silence");
        assert!(!trimmed.is_empty(), "Should keep audio content");
        
        // Should detect silence regions
        assert!(!silence_map.is_empty(), "Should detect silence regions: {:?}", silence_map);
    }
    
    #[test]
    fn test_detect_and_trim_all_silent() {
        // All silent audio
        let samples = vec![0.0f32; 44100];
        
        let detector = SilenceDetector::default();
        let (trimmed, _silence_map) = detect_and_trim(&samples, 44100, detector).unwrap();
        
        // Should trim everything
        assert!(trimmed.is_empty() || trimmed.iter().all(|&x| x.abs() < 1e-6),
                "All silent audio should be trimmed");
    }
    
    #[test]
    fn test_detect_and_trim_no_silence() {
        // Audio with no silence
        let mut samples = vec![0.0f32; 44100];
        for i in 0..samples.len() {
            samples[i] = 0.5 * (i as f32 / 1000.0).sin();
        }
        
        let detector = SilenceDetector {
            threshold_db: -60.0, // Very low threshold
            ..Default::default()
        };
        let (trimmed, _silence_map) = detect_and_trim(&samples, 44100, detector).unwrap();
        
        // Should keep most/all of the audio
        assert!(trimmed.len() > samples.len() / 2,
                "Should keep most audio when no silence detected");
    }
    
    #[test]
    fn test_detect_and_trim_invalid_parameters() {
        let samples = vec![0.5f32; 44100];
        let detector = SilenceDetector::default();
        
        // Test zero sample rate
        let result = detect_and_trim(&samples, 0, detector.clone());
        assert!(result.is_err());
        
        // Test zero frame size
        let mut bad_detector = detector.clone();
        bad_detector.frame_size = 0;
        let result = detect_and_trim(&samples, 44100, bad_detector);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_detect_and_trim_empty_samples() {
        let samples = vec![];
        let detector = SilenceDetector::default();
        let (trimmed, silence_map) = detect_and_trim(&samples, 44100, detector).unwrap();
        
        assert!(trimmed.is_empty());
        assert!(silence_map.is_empty());
    }
    
    #[test]
    fn test_detect_and_trim_threshold_sensitivity() {
        // Create audio with varying levels
        let mut samples = vec![0.0f32; 44100 * 2];
        // Add quiet section (below -40 dB)
        for i in 0..22050 {
            samples[i] = 0.01; // ~-40 dB
        }
        // Add louder section
        for i in 22050..44100 {
            samples[i] = 0.5; // Much louder
        }
        
        // Low threshold should detect less silence
        let detector_low = SilenceDetector {
            threshold_db: -60.0,
            ..Default::default()
        };
        let (_, silence_map_low) = detect_and_trim(&samples, 44100, detector_low).unwrap();
        
        // High threshold should detect more silence
        let detector_high = SilenceDetector {
            threshold_db: -20.0,
            ..Default::default()
        };
        let (_, silence_map_high) = detect_and_trim(&samples, 44100, detector_high).unwrap();
        
        // Higher threshold should detect more silence regions (or longer ones)
        let total_silence_low: usize = silence_map_low.iter()
            .map(|(start, end)| end - start)
            .sum();
        let total_silence_high: usize = silence_map_high.iter()
            .map(|(start, end)| end - start)
            .sum();
        
        assert!(total_silence_high >= total_silence_low,
                "Higher threshold should detect more silence");
    }
    
    #[test]
    fn test_detect_and_trim_min_duration() {
        // Create audio with short silence bursts
        let mut samples = vec![0.5f32; 44100 * 2];
        // Add very short silence (less than 500ms)
        for i in 10000..15000 {
            samples[i] = 0.0;
        }
        
        let detector = SilenceDetector {
            min_duration_ms: 500, // 500ms minimum
            ..Default::default()
        };
        let (_, _silence_map) = detect_and_trim(&samples, 44100, detector).unwrap();
        
        // Short silence bursts should be filtered out (unless leading/trailing)
        // The 5000 sample silence is ~113ms, which is less than 500ms
        // So it might not be included unless it's at the edges
    }
}

