//! Channel mixing utilities (stereo to mono conversion)
//!
//! Converts stereo audio to mono using various mixing strategies.
//!
//! # Mixing Modes
//!
//! - **Mono**: Simple average `(L + R) / 2` - preserves overall energy
//! - **MidSide**: Extracts mid component `(L + R) / 2` - same as Mono but conceptually different
//! - **Dominant**: Keeps the louder channel per sample - preserves maximum amplitude
//! - **Center**: Extracts center image - emphasizes common content between channels
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::preprocessing::channel_mixer::{stereo_to_mono, ChannelMixMode};
//!
//! let left = vec![0.5f32; 44100];
//! let right = vec![0.3f32; 44100];
//! let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono)?;
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;

/// Channel mixing mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixMode {
    /// Simple average: (L + R) / 2
    /// Preserves overall energy and is the most common method
    Mono,
    /// Mid-side: (L + R) / 2 (extracts mid component)
    /// Conceptually extracts the "mid" from MS encoding, but mathematically same as Mono
    MidSide,
    /// Keep louder channel per sample
    /// Preserves maximum amplitude: max(|L|, |R|) * sign of louder channel
    Dominant,
    /// Center image only
    /// Emphasizes content common to both channels: (L + R) / 2
    /// Same as Mono but conceptually emphasizes center-panned content
    Center,
}

/// Convert stereo to mono
///
/// This function converts stereo audio (left and right channels) to mono
/// using the specified mixing mode. Different modes are useful for different
/// analysis scenarios.
///
/// # Arguments
///
/// * `left` - Left channel samples (normalized to [-1.0, 1.0])
/// * `right` - Right channel samples (normalized to [-1.0, 1.0])
/// * `mode` - Mixing mode to use
///
/// # Returns
///
/// Mono samples as `Vec<f32>`
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Left and right channels have different lengths
/// - Input is empty
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::preprocessing::channel_mixer::{stereo_to_mono, ChannelMixMode};
///
/// let left = vec![0.5f32; 44100];
/// let right = vec![0.3f32; 44100];
///
/// // Simple average (most common)
/// let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono)?;
///
/// // Keep louder channel
/// let dominant = stereo_to_mono(&left, &right, ChannelMixMode::Dominant)?;
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn stereo_to_mono(
    left: &[f32],
    right: &[f32],
    mode: ChannelMixMode,
) -> Result<Vec<f32>, AnalysisError> {
    // Validate inputs
    if left.len() != right.len() {
        return Err(AnalysisError::InvalidInput(
            format!("Left and right channels must have same length: left={}, right={}", 
                    left.len(), right.len())
        ));
    }
    
    if left.is_empty() {
        return Ok(Vec::new());
    }
    
    log::debug!("Converting stereo to mono: {} samples using {:?}", left.len(), mode);
    
    let mono = match mode {
        ChannelMixMode::Mono => {
            // Simple average: (L + R) / 2
            left.iter()
                .zip(right.iter())
                .map(|(&l, &r)| (l + r) * 0.5)
                .collect()
        }
        ChannelMixMode::MidSide => {
            // Mid-side: extract mid component (L + R) / 2
            // Mathematically same as Mono, but conceptually extracts "mid" from MS encoding
            left.iter()
                .zip(right.iter())
                .map(|(&l, &r)| (l + r) * 0.5)
                .collect()
        }
        ChannelMixMode::Dominant => {
            // Keep louder channel: max(|L|, |R|) * sign of louder channel
            left.iter()
                .zip(right.iter())
                .map(|(&l, &r)| {
                    let abs_l = l.abs();
                    let abs_r = r.abs();
                    if abs_l >= abs_r {
                        l
                    } else {
                        r
                    }
                })
                .collect()
        }
        ChannelMixMode::Center => {
            // Center image: (L + R) / 2
            // Emphasizes content common to both channels (center-panned)
            // Same as Mono but conceptually different
            left.iter()
                .zip(right.iter())
                .map(|(&l, &r)| (l + r) * 0.5)
                .collect()
        }
    };
    
    Ok(mono)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_stereo_to_mono_simple_average() {
        let left = vec![1.0f32, 0.5f32, 0.0f32];
        let right = vec![0.0f32, 0.5f32, 1.0f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.5).abs() < 1e-6); // (1.0 + 0.0) / 2 = 0.5
        assert!((mono[1] - 0.5).abs() < 1e-6); // (0.5 + 0.5) / 2 = 0.5
        assert!((mono[2] - 0.5).abs() < 1e-6); // (0.0 + 1.0) / 2 = 0.5
    }
    
    #[test]
    fn test_stereo_to_mono_midside() {
        let left = vec![1.0f32, 0.5f32];
        let right = vec![0.0f32, 0.5f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::MidSide).unwrap();
        let mono_simple = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        
        // MidSide should produce same result as Mono
        assert_eq!(mono, mono_simple);
    }
    
    #[test]
    fn test_stereo_to_mono_center() {
        let left = vec![1.0f32, 0.5f32];
        let right = vec![0.0f32, 0.5f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Center).unwrap();
        let mono_simple = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        
        // Center should produce same result as Mono
        assert_eq!(mono, mono_simple);
    }
    
    #[test]
    fn test_stereo_to_mono_dominant() {
        let left = vec![1.0f32, 0.3f32, -0.8f32];
        let right = vec![0.2f32, 0.9f32, -0.1f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Dominant).unwrap();
        
        assert_eq!(mono.len(), 3);
        // First sample: |1.0| > |0.2|, so keep left
        assert!((mono[0] - 1.0).abs() < 1e-6);
        // Second sample: |0.9| > |0.3|, so keep right
        assert!((mono[1] - 0.9).abs() < 1e-6);
        // Third sample: |-0.8| > |-0.1|, so keep left
        assert!((mono[2] - (-0.8)).abs() < 1e-6);
    }
    
    #[test]
    fn test_stereo_to_mono_dominant_tie() {
        // When both channels have same absolute value, keep the first (left)
        let left = vec![0.5f32, -0.5f32];
        let right = vec![0.5f32, 0.5f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Dominant).unwrap();
        
        assert_eq!(mono.len(), 2);
        // First: tie, keep left (0.5)
        assert!((mono[0] - 0.5).abs() < 1e-6);
        // Second: tie, keep left (-0.5)
        assert!((mono[1] - (-0.5)).abs() < 1e-6);
    }
    
    #[test]
    fn test_stereo_to_mono_different_lengths() {
        let left = vec![1.0f32, 0.5f32];
        let right = vec![0.0f32];
        
        let result = stereo_to_mono(&left, &right, ChannelMixMode::Mono);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_stereo_to_mono_empty() {
        let left = vec![];
        let right = vec![];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        assert!(mono.is_empty());
    }
    
    #[test]
    fn test_stereo_to_mono_large_input() {
        // Test with larger input for performance
        let len = 44100 * 30; // 30 seconds at 44.1kHz
        let left: Vec<f32> = (0..len).map(|i| (i as f32 / 1000.0).sin()).collect();
        let right: Vec<f32> = (0..len).map(|i| (i as f32 / 1000.0).cos()).collect();
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        
        assert_eq!(mono.len(), len);
        // Verify it's actually averaging
        for i in 0..100.min(len) {
            let expected = (left[i] + right[i]) * 0.5;
            assert!((mono[i] - expected).abs() < 1e-6,
                    "Mismatch at index {}: expected {}, got {}", i, expected, mono[i]);
        }
    }
    
    #[test]
    fn test_stereo_to_mono_all_modes() {
        let left = vec![0.8f32, 0.2f32, -0.5f32];
        let right = vec![0.2f32, 0.8f32, -0.3f32];
        
        let mono = stereo_to_mono(&left, &right, ChannelMixMode::Mono).unwrap();
        let midside = stereo_to_mono(&left, &right, ChannelMixMode::MidSide).unwrap();
        let center = stereo_to_mono(&left, &right, ChannelMixMode::Center).unwrap();
        let dominant = stereo_to_mono(&left, &right, ChannelMixMode::Dominant).unwrap();
        
        // Mono, MidSide, and Center should all be the same
        assert_eq!(mono, midside);
        assert_eq!(mono, center);
        
        // Dominant should be different (keeps louder channel)
        assert_ne!(mono, dominant);
        
        // Verify dominant keeps the louder channel
        assert!((dominant[0] - 0.8).abs() < 1e-6); // |0.8| > |0.2|
        assert!((dominant[1] - 0.8).abs() < 1e-6); // |0.8| > |0.2|
        assert!((dominant[2] - (-0.5)).abs() < 1e-6); // |-0.5| > |-0.3|
    }
}

