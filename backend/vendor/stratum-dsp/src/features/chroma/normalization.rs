//! Chroma normalization strategies
//!
//! Provides normalization methods for chroma vectors to improve key detection accuracy.
//!
//! # Reference
//!
//! Müller, M., & Ewert, S. (2010). Chroma Toolbox: MATLAB Implementations for Extracting
//! Variants of Chroma-Based Audio Features. *Proceedings of the International Society for
//! Music Information Retrieval Conference*.

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Sharpen chroma vector to emphasize prominent semitones
///
/// Applies a power function to each element before renormalization, which
/// increases the contrast between strong and weak semitone classes.
///
/// # Arguments
///
/// * `chroma` - 12-element chroma vector (should be L2 normalized)
/// * `power` - Sharpening power (typically 1.5 or 2.0)
///   - power = 1.0: no change
///   - power > 1.0: increases contrast (emphasizes peaks)
///   - power < 1.0: decreases contrast (smooths)
///
/// # Returns
///
/// Sharpened chroma vector (L2 normalized)
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::chroma::normalization::sharpen_chroma;
///
/// let chroma = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.0];
/// let sharpened = sharpen_chroma(&chroma, 2.0);
/// // Stronger semitones are now more prominent
/// # assert_eq!(sharpened.len(), 12);
/// ```
pub fn sharpen_chroma(chroma: &[f32], power: f32) -> Vec<f32> {
    if chroma.is_empty() {
        return vec![];
    }
    
    log::debug!("Sharpening chroma with power {}", power);
    
    // Apply power function element-wise
    let mut sharpened: Vec<f32> = chroma
        .iter()
        .map(|&x| x.powf(power))
        .collect();
    
    // L2 normalize
    let norm: f32 = sharpened.iter().map(|&x| x * x).sum::<f32>().sqrt();
    
    if norm > EPSILON {
        for x in &mut sharpened {
            *x /= norm;
        }
    } else {
        // If norm is too small, return uniform distribution
        let uniform = 1.0 / (chroma.len() as f32).sqrt();
        sharpened.fill(uniform);
    }
    
    sharpened
}

/// L2 normalize a chroma vector
///
/// Normalizes the vector to unit length (L2 norm = 1.0).
///
/// # Arguments
///
/// * `chroma` - 12-element chroma vector
///
/// # Returns
///
/// L2 normalized chroma vector
pub fn l2_normalize_chroma(chroma: &[f32]) -> Vec<f32> {
    if chroma.is_empty() {
        return vec![];
    }
    
    let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
    
    if norm > EPSILON {
        chroma.iter().map(|&x| x / norm).collect()
    } else {
        // If norm is too small, return uniform distribution
        let uniform = 1.0 / (chroma.len() as f32).sqrt();
        vec![uniform; chroma.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sharpen_chroma() {
        let chroma = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.0];
        let sharpened = sharpen_chroma(&chroma, 2.0);
        
        assert_eq!(sharpened.len(), 12);
        
        // Check normalization
        let norm: f32 = sharpened.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
        
        // Check that stronger values are more prominent
        // The peak at index 5 (0.6) should be even stronger after sharpening
        assert!(sharpened[5] > chroma[5] || chroma[5] < 0.1);
    }
    
    #[test]
    fn test_sharpen_chroma_power_one() {
        let chroma = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.0];
        let sharpened = sharpen_chroma(&chroma, 1.0);
        
        // Power 1.0 should preserve relative magnitudes (after normalization)
        let norm_orig: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
        let normalized_orig: Vec<f32> = chroma.iter().map(|&x| x / norm_orig).collect();
        
        // Should be approximately equal (within numerical precision)
        for (a, b) in normalized_orig.iter().zip(sharpened.iter()) {
            assert!((a - b).abs() < 0.01);
        }
    }
    
    #[test]
    fn test_sharpen_chroma_empty() {
        let result = sharpen_chroma(&[], 2.0);
        assert_eq!(result.len(), 0);
    }
    
    #[test]
    fn test_l2_normalize_chroma() {
        let chroma = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0];
        let normalized = l2_normalize_chroma(&chroma);
        
        assert_eq!(normalized.len(), 12);
        
        // Check L2 norm is 1.0
        let norm: f32 = normalized.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }
    
    #[test]
    fn test_l2_normalize_chroma_empty() {
        let result = l2_normalize_chroma(&[]);
        assert_eq!(result.len(), 0);
    }
    
    #[test]
    fn test_l2_normalize_chroma_zero() {
        let chroma = vec![0.0; 12];
        let normalized = l2_normalize_chroma(&chroma);
        
        // Should return uniform distribution when input is all zeros
        let expected = 1.0 / (12.0f32).sqrt();
        for &x in &normalized {
            assert!((x - expected).abs() < 0.01);
        }
    }
}
