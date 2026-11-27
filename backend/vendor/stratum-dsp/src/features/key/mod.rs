//! Key detection modules
//!
//! Detect musical key using:
//! - Krumhansl-Kessler templates (24 keys)
//! - Template matching
//! - Key clarity scoring

pub mod detector;
pub mod key_changes;
pub mod key_clarity;
pub mod templates;

pub use detector::{detect_key, detect_key_ensemble, detect_key_multi_scale, detect_key_weighted, detect_key_weighted_mode_heuristic};
pub use key_changes::{detect_key_changes, KeyChange, KeyChangeResult};
pub use key_clarity::compute_key_clarity;
pub use templates::KeyTemplates;

use crate::analysis::result::Key;

/// Key detection result
#[derive(Debug, Clone)]
pub struct KeyDetectionResult {
    /// Detected key (best match)
    pub key: Key,
    
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
    
    /// All 24 key scores (ranked, highest first)
    pub all_scores: Vec<(Key, f32)>,
    
    /// Top N keys with scores (default: top 3)
    /// Useful for ambiguous cases or DJ key mixing
    pub top_keys: Vec<(Key, f32)>,
}

