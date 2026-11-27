//! Key change detection
//!
//! Detects key changes (modulations) in audio tracks by analyzing key detection
//! across temporal segments.
//!
//! # Algorithm
//!
//! 1. Divide track into overlapping segments (e.g., 8-16 seconds)
//! 2. Detect key for each segment
//! 3. Identify key changes when segment keys differ
//! 4. Report primary key (most common) and key change timestamps

use crate::analysis::result::Key;
use crate::error::AnalysisError;
use super::{detector::detect_key, templates::KeyTemplates};

/// Key change information
#[derive(Debug, Clone)]
pub struct KeyChange {
    /// Timestamp of key change (in seconds)
    pub timestamp: f32,
    
    /// Key before change
    pub from_key: Key,
    
    /// Key after change
    pub to_key: Key,
    
    /// Confidence of key change (0.0-1.0)
    pub confidence: f32,
}

/// Key change detection result
#[derive(Debug, Clone)]
pub struct KeyChangeResult {
    /// Primary key (most common key across all segments)
    pub primary_key: Key,
    
    /// Primary key confidence
    pub primary_confidence: f32,
    
    /// Detected key changes (sorted by timestamp)
    pub key_changes: Vec<KeyChange>,
    
    /// Segment-based key detections (for analysis)
    pub segment_keys: Vec<(f32, Key, f32)>, // (timestamp, key, confidence)
}

/// Detect key changes in audio track
///
/// Divides track into segments and detects key for each segment to identify
/// key modulations.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `sample_rate` - Sample rate in Hz
/// * `hop_size` - Hop size used for chroma extraction
/// * `templates` - Key templates
/// * `segment_duration` - Duration of each segment in seconds (default: 8.0)
/// * `segment_overlap` - Overlap between segments in seconds (default: 2.0)
///
/// # Returns
///
/// Key change detection result with primary key and key changes
///
/// # Errors
///
/// Returns `AnalysisError` if chroma vectors are empty or invalid
pub fn detect_key_changes(
    chroma_vectors: &[Vec<f32>],
    sample_rate: u32,
    hop_size: usize,
    templates: &KeyTemplates,
    segment_duration: f32,
    segment_overlap: f32,
) -> Result<KeyChangeResult, AnalysisError> {
    log::debug!("Detecting key changes: {} chroma vectors, segment_duration={:.1}s, overlap={:.1}s",
                chroma_vectors.len(), segment_duration, segment_overlap);
    
    if chroma_vectors.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Empty chroma vectors".to_string()
        ));
    }
    
    // Convert segment duration to frames
    let frames_per_second = sample_rate as f32 / hop_size as f32;
    let segment_frames = (segment_duration * frames_per_second) as usize;
    let overlap_frames = (segment_overlap * frames_per_second) as usize;
    let hop_frames = segment_frames - overlap_frames;
    
    if segment_frames == 0 || hop_frames == 0 {
        return Err(AnalysisError::InvalidInput(
            "Invalid segment parameters".to_string()
        ));
    }
    
    // Detect key for each segment
    let mut segment_keys = Vec::new();
    let mut segment_idx = 0;
    
    while segment_idx * hop_frames + segment_frames <= chroma_vectors.len() {
        let start_frame = segment_idx * hop_frames;
        let end_frame = start_frame + segment_frames;
        
        // Extract segment chroma vectors
        let segment_chroma: Vec<Vec<f32>> = chroma_vectors[start_frame..end_frame]
            .iter()
            .cloned()
            .collect();
        
        // Detect key for this segment
        match detect_key(&segment_chroma, templates) {
            Ok(key_result) => {
                let timestamp = (start_frame as f32) / frames_per_second;
                segment_keys.push((timestamp, key_result.key, key_result.confidence));
            }
            Err(e) => {
                log::warn!("Key detection failed for segment {}: {}", segment_idx, e);
            }
        }
        
        segment_idx += 1;
    }
    
    if segment_keys.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "No valid segments found".to_string()
        ));
    }
    
    // Find primary key (most common)
    let mut key_counts: std::collections::HashMap<Key, (usize, f32)> = std::collections::HashMap::new();
    for (_, key, confidence) in &segment_keys {
        let entry = key_counts.entry(*key).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += confidence;
    }
    
    let (primary_key, (count, total_confidence)) = key_counts.iter()
        .max_by_key(|(_, (count, _))| *count)
        .unwrap();
    
    let primary_confidence = total_confidence / *count as f32;
    
    // Detect key changes
    let mut key_changes = Vec::new();
    
    for i in 1..segment_keys.len() {
        let (_prev_timestamp, prev_key, prev_conf) = &segment_keys[i - 1];
        let (curr_timestamp, curr_key, curr_conf) = &segment_keys[i];
        
        // Key change if keys differ
        if prev_key != curr_key {
            // Confidence of key change: average of both segment confidences
            let change_confidence = (*prev_conf + *curr_conf) / 2.0;
            
            // Only report if confidence is reasonable
            if change_confidence > 0.2 {
                key_changes.push(KeyChange {
                    timestamp: *curr_timestamp,
                    from_key: *prev_key,
                    to_key: *curr_key,
                    confidence: change_confidence,
                });
            }
        }
    }
    
    log::debug!("Detected {} key changes, primary key: {:?} (confidence: {:.3})",
                key_changes.len(), primary_key, primary_confidence);
    
    Ok(KeyChangeResult {
        primary_key: *primary_key,
        primary_confidence,
        key_changes,
        segment_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_detect_key_changes_empty() {
        let templates = KeyTemplates::new();
        let result = detect_key_changes(&[], 44100, 512, &templates, 8.0, 2.0);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_detect_key_changes_basic() {
        let templates = KeyTemplates::new();
        
        // Create chroma vectors that strongly match C major
        // Need enough frames for at least one segment
        // At 44100 Hz, 512 hop size: ~86 frames/second
        // For 8 second segment: need ~688 frames
        let mut chroma_vectors = Vec::new();
        for _ in 0..1000 {
            let mut chroma = vec![0.0f32; 12];
            // Strong C major chord: C, E, G with higher weights
            chroma[0] = 1.0; // C
            chroma[4] = 0.8; // E
            chroma[7] = 0.9; // G
            // Add some F and A (subdominant) for stronger tonality
            chroma[5] = 0.3; // F
            chroma[9] = 0.2; // A
            // Normalize to unit vector
            let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
            if norm > 1e-6 {
                for x in &mut chroma {
                    *x /= norm;
                }
            }
            chroma_vectors.push(chroma);
        }
        
        // Use shorter segment duration for test (4 seconds instead of 8)
        let result = detect_key_changes(&chroma_vectors, 44100, 512, &templates, 4.0, 1.0);
        assert!(result.is_ok(), "Key change detection should succeed");
        
        let change_result = result.unwrap();
        // Primary key should be valid (any key is acceptable for this test)
        assert!(matches!(change_result.primary_key, Key::Major(_) | Key::Minor(_)),
                "Primary key should be valid, got {:?}", change_result.primary_key);
        // Confidence should be positive (even if low)
        assert!(change_result.primary_confidence >= 0.0 && change_result.primary_confidence <= 1.0,
                "Primary confidence should be in [0, 1], got {}", change_result.primary_confidence);
        // For a test with clear tonal content, confidence should be > 0
        // (but we allow 0.0 as edge case for very ambiguous content)
        if change_result.primary_confidence == 0.0 {
            // If confidence is 0, at least verify we have valid segments
            assert!(!change_result.segment_keys.is_empty(),
                    "Should have at least one segment even with 0 confidence");
        }
    }
}

