//! Temporal chroma smoothing
//!
//! Smooths chroma vectors over time to reduce frame-to-frame variance and improve
//! key detection stability.
//!
//! # Reference
//!
//! Müller, M., & Ewert, S. (2010). Chroma Toolbox: MATLAB Implementations for Extracting
//! Variants of Chroma-Based Audio Features. *Proceedings of the International Society for
//! Music Information Retrieval Conference*.

/// Smooth chroma vectors over time using median filtering
///
/// Applies a median filter across time for each semitone class independently.
/// This reduces frame-to-frame variance while preserving sharp transitions.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `window_size` - Smoothing window size in frames (e.g., 5)
///   - Must be odd (if even, will be incremented by 1)
///   - Typical values: 3, 5, 7
///
/// # Returns
///
/// Smoothed chroma vectors (same length as input)
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::chroma::smoothing::smooth_chroma;
///
/// let chroma_vectors = vec![vec![0.1; 12]; 100]; // 100 frames
/// let smoothed = smooth_chroma(&chroma_vectors, 5);
/// assert_eq!(smoothed.len(), 100);
/// ```
pub fn smooth_chroma(
    chroma_vectors: &[Vec<f32>],
    window_size: usize,
) -> Vec<Vec<f32>> {
    log::debug!("Smoothing {} chroma vectors with window size {}",
                chroma_vectors.len(), window_size);
    
    if chroma_vectors.is_empty() {
        return vec![];
    }
    
    if window_size == 0 || window_size == 1 {
        return chroma_vectors.to_vec();
    }
    
    // Ensure window size is odd
    let window_size = if window_size % 2 == 0 {
        window_size + 1
    } else {
        window_size
    };
    
    let half_window = window_size / 2;
    let n_frames = chroma_vectors.len();
    let n_semitones = chroma_vectors[0].len();
    
    let mut smoothed = Vec::with_capacity(n_frames);
    
    for frame_idx in 0..n_frames {
        let mut smoothed_frame = vec![0.0f32; n_semitones];
        
        for semitone_idx in 0..n_semitones {
            // Collect values in window for this semitone class
            let mut window_values = Vec::new();
            
            for offset in 0..window_size {
                let frame_idx_offset = frame_idx as i32 + (offset as i32 - half_window as i32);
                
                if frame_idx_offset >= 0 && (frame_idx_offset as usize) < n_frames {
                    window_values.push(chroma_vectors[frame_idx_offset as usize][semitone_idx]);
                }
            }
            
            // Compute median
            if !window_values.is_empty() {
                window_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let median_idx = window_values.len() / 2;
                smoothed_frame[semitone_idx] = window_values[median_idx];
            } else {
                smoothed_frame[semitone_idx] = chroma_vectors[frame_idx][semitone_idx];
            }
        }
        
        smoothed.push(smoothed_frame);
    }
    
    smoothed
}

/// Smooth chroma vectors over time using average filtering
///
/// Applies a moving average filter across time for each semitone class independently.
/// This provides smoother results than median filtering but may blur sharp transitions.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `window_size` - Smoothing window size in frames (e.g., 5)
///
/// # Returns
///
/// Smoothed chroma vectors (same length as input)
pub fn smooth_chroma_average(
    chroma_vectors: &[Vec<f32>],
    window_size: usize,
) -> Vec<Vec<f32>> {
    log::debug!("Smoothing {} chroma vectors with average filter, window size {}",
                chroma_vectors.len(), window_size);
    
    if chroma_vectors.is_empty() {
        return vec![];
    }
    
    if window_size == 0 || window_size == 1 {
        return chroma_vectors.to_vec();
    }
    
    let half_window = window_size / 2;
    let n_frames = chroma_vectors.len();
    let n_semitones = chroma_vectors[0].len();
    
    let mut smoothed = Vec::with_capacity(n_frames);
    
    for frame_idx in 0..n_frames {
        let mut smoothed_frame = vec![0.0f32; n_semitones];
        
        for semitone_idx in 0..n_semitones {
            let mut sum = 0.0f32;
            let mut count = 0;
            
            // Sum values in window
            for offset in 0..window_size {
                let frame_idx_offset = frame_idx as i32 + (offset as i32 - half_window as i32);
                
                if frame_idx_offset >= 0 && (frame_idx_offset as usize) < n_frames {
                    sum += chroma_vectors[frame_idx_offset as usize][semitone_idx];
                    count += 1;
                }
            }
            
            // Average
            if count > 0 {
                smoothed_frame[semitone_idx] = sum / count as f32;
            } else {
                smoothed_frame[semitone_idx] = chroma_vectors[frame_idx][semitone_idx];
            }
        }
        
        smoothed.push(smoothed_frame);
    }
    
    smoothed
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_smooth_chroma_empty() {
        let result = smooth_chroma(&[], 5);
        assert_eq!(result.len(), 0);
    }
    
    #[test]
    fn test_smooth_chroma_single_frame() {
        let chroma_vectors = vec![vec![0.1; 12]];
        let smoothed = smooth_chroma(&chroma_vectors, 5);
        assert_eq!(smoothed.len(), 1);
        assert_eq!(smoothed[0].len(), 12);
    }
    
    #[test]
    fn test_smooth_chroma_basic() {
        // Create chroma vectors with some variation
        let mut chroma_vectors = Vec::new();
        for i in 0..10 {
            let mut chroma = vec![0.0f32; 12];
            chroma[i % 12] = 1.0;
            chroma_vectors.push(chroma);
        }
        
        let smoothed = smooth_chroma(&chroma_vectors, 3);
        assert_eq!(smoothed.len(), 10);
        
        for chroma in &smoothed {
            assert_eq!(chroma.len(), 12);
        }
    }
    
    #[test]
    fn test_smooth_chroma_window_size_one() {
        let chroma_vectors = vec![vec![0.1; 12]; 5];
        let smoothed = smooth_chroma(&chroma_vectors, 1);
        assert_eq!(smoothed.len(), chroma_vectors.len());
    }
    
    #[test]
    fn test_smooth_chroma_average() {
        let mut chroma_vectors = Vec::new();
        for i in 0..10 {
            let mut chroma = vec![0.0f32; 12];
            chroma[i % 12] = 1.0;
            chroma_vectors.push(chroma);
        }
        
        let smoothed = smooth_chroma_average(&chroma_vectors, 3);
        assert_eq!(smoothed.len(), 10);
        
        for chroma in &smoothed {
            assert_eq!(chroma.len(), 12);
        }
    }
    
    #[test]
    fn test_smooth_chroma_even_window_size() {
        // Window size 4 should be converted to 5
        let chroma_vectors = vec![vec![0.1; 12]; 10];
        let smoothed = smooth_chroma(&chroma_vectors, 4);
        assert_eq!(smoothed.len(), 10);
    }
}
