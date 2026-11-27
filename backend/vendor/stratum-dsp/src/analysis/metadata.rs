//! Analysis metadata structures

use super::result::AnalysisFlag;

/// Analysis metadata
#[derive(Debug, Clone)]
pub struct AnalysisMetadata {
    /// Algorithm version
    pub algorithm_version: String,
    
    /// Onset method consensus score
    pub onset_method_consensus: f32,
    
    /// Methods used
    pub methods_used: Vec<String>,
    
    /// Analysis flags
    pub flags: Vec<AnalysisFlag>,
}

impl Default for AnalysisMetadata {
    fn default() -> Self {
        Self {
            algorithm_version: env!("CARGO_PKG_VERSION").to_string(),
            onset_method_consensus: 0.0,
            methods_used: vec![],
            flags: vec![],
        }
    }
}

