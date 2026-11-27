//! Onset detection modules
//!
//! Multiple onset detection methods with consensus voting:
//! - Energy flux
//! - Spectral flux
//! - High-frequency content (HFC)
//! - Harmonic-percussive source separation (HPSS)
//! - Consensus voting

pub mod consensus;
pub mod energy_flux;
pub mod hfc;
pub mod hpss;
pub mod spectral_flux;
// Thresholding utilities (median/MAD, percentile).
//
// NOTE: Not currently wired into the main onset detectors; kept as a reference implementation
// with proper citation for future tuning work.
pub mod threshold;

/// Onset candidate with confidence
#[derive(Debug, Clone)]
pub struct OnsetCandidate {
    /// Onset time in samples
    pub time_samples: usize,
    
    /// Onset time in seconds
    pub time_seconds: f32,
    
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
    
    /// Number of methods that detected this onset
    pub voted_by: u32,
}

