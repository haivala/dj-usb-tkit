//! Feature extraction modules
//!
//! This module contains all feature extraction algorithms:
//! - Onset detection (4 methods + consensus)
//! - Period estimation (BPM detection)
//! - Beat tracking (HMM + Bayesian)
//! - Chroma extraction
//! - Key detection

pub mod beat_tracking;
pub mod chroma;
pub mod key;
pub mod onset;
pub mod period;

