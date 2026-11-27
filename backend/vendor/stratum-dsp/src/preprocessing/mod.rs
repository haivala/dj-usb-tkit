//! Audio preprocessing modules
//!
//! This module contains utilities for preparing audio for analysis:
//! - Normalization (peak, RMS, LUFS)
//! - Silence detection and trimming
//! - Channel mixing (stereo to mono)

pub mod channel_mixer;
pub mod normalization;
pub mod silence;

