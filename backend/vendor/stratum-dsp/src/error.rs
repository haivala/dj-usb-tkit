//! Error types for the audio analysis engine

use std::fmt;

/// Errors that can occur during audio analysis
#[derive(Debug, Clone)]
pub enum AnalysisError {
    /// Invalid input parameters
    InvalidInput(String),
    
    /// Audio decoding error
    DecodingError(String),
    
    /// Processing error during analysis
    ProcessingError(String),
    
    /// Feature not yet implemented
    NotImplemented(String),
    
    /// Numerical error (overflow, underflow, etc.)
    NumericalError(String),
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnalysisError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            AnalysisError::DecodingError(msg) => write!(f, "Decoding error: {}", msg),
            AnalysisError::ProcessingError(msg) => write!(f, "Processing error: {}", msg),
            AnalysisError::NotImplemented(msg) => write!(f, "Not implemented: {}", msg),
            AnalysisError::NumericalError(msg) => write!(f, "Numerical error: {}", msg),
        }
    }
}

impl std::error::Error for AnalysisError {}

