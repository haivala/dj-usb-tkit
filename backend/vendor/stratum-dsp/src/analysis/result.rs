//! Analysis result types

use serde::{Deserialize, Serialize};

/// Musical key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    /// Major key (0 = C, 1 = C#, ..., 11 = B)
    Major(u32),
    /// Minor key (0 = C, 1 = C#, ..., 11 = B)
    Minor(u32),
}

impl Key {
    /// Get key name in musical notation (e.g., "C", "Am", "F#", "D#m")
    ///
    /// Returns standard musical notation:
    /// - Major keys: note name only (e.g., "C", "C#", "D", "F#")
    /// - Minor keys: note name + "m" (e.g., "Am", "C#m", "Dm", "F#m")
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::Major(0).name(), "C");
    /// assert_eq!(Key::Major(6).name(), "F#");
    /// assert_eq!(Key::Minor(9).name(), "Am");
    /// assert_eq!(Key::Minor(1).name(), "C#m");
    /// ```
    pub fn name(&self) -> String {
        let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        match self {
            Key::Major(i) => note_names[*i as usize % 12].to_string(),
            Key::Minor(i) => format!("{}m", note_names[*i as usize % 12]),
        }
    }
    
    /// Get key in DJ standard numerical notation (e.g., "1A", "2B", "12A")
    ///
    /// Uses the circle of fifths mapping popularized in DJ software:
    /// - Major keys: 1A-12A (1A = C, 2A = G, 3A = D, ..., 12A = F)
    /// - Minor keys: 1B-12B (1B = Am, 2B = Em, 3B = Bm, ..., 12B = Dm)
    ///
    /// The numbering follows the circle of fifths (up a fifth each step).
    /// Keys are displayed in standard musical notation by default (e.g., "C", "Am", "F#").
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::Major(0).numerical(), "1A");   // C
    /// assert_eq!(Key::Major(7).numerical(), "2A");   // G
    /// assert_eq!(Key::Minor(9).numerical(), "1B");   // Am
    /// assert_eq!(Key::Minor(4).numerical(), "2B");   // Em
    /// ```
    pub fn numerical(&self) -> String {
        // Circle of fifths mapping: C=0, G=7, D=2, A=9, E=4, B=11, F#=6, C#=1, G#=8, D#=3, A#=10, F=5
        // This maps to: 1A, 2A, 3A, 4A, 5A, 6A, 7A, 8A, 9A, 10A, 11A, 12A
        let circle_of_fifths_major = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5]; // C, G, D, A, E, B, F#, C#, G#, D#, A#, F
        
        match self {
            Key::Major(i) => {
                let key_idx = *i as usize % 12;
                // Find position in circle of fifths
                let position = circle_of_fifths_major.iter()
                    .position(|&x| x == key_idx)
                    .unwrap_or(0);
                format!("{}A", position + 1)
            }
            Key::Minor(i) => {
                let key_idx = *i as usize % 12;
                // Minor keys follow the same circle of fifths pattern
                // Relative minor of 1A (C) is 1B (Am), relative minor of 2A (G) is 2B (Em), etc.
                let circle_of_fifths_minor = [9, 4, 11, 6, 1, 8, 3, 10, 5, 0, 7, 2]; // Am, Em, Bm, F#m, C#m, G#m, D#m, A#m, Fm, Cm, Gm, Dm
                let position = circle_of_fifths_minor.iter()
                    .position(|&x| x == key_idx)
                    .unwrap_or(0);
                format!("{}B", position + 1)
            }
        }
    }
    
    /// Get key from DJ standard numerical notation
    ///
    /// Converts numerical notation (e.g., "1A", "2B", "12A") back to a Key.
    ///
    /// # Arguments
    ///
    /// * `notation` - Numerical key notation (e.g., "1A", "2B", "12A")
    ///
    /// # Returns
    ///
    /// `Some(Key)` if valid, `None` if invalid format
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::from_numerical("1A"), Some(Key::Major(0)));   // C
    /// assert_eq!(Key::from_numerical("2A"), Some(Key::Major(7)));   // G
    /// assert_eq!(Key::from_numerical("1B"), Some(Key::Minor(9)));   // Am
    /// assert_eq!(Key::from_numerical("2B"), Some(Key::Minor(4)));   // Em
    /// assert_eq!(Key::from_numerical("0A"), None);  // Invalid
    /// assert_eq!(Key::from_numerical("13A"), None); // Invalid
    /// ```
    pub fn from_numerical(notation: &str) -> Option<Self> {
        if notation.len() < 2 {
            return None;
        }
        
        let (num_str, suffix) = notation.split_at(notation.len() - 1);
        let num: u32 = num_str.parse().ok()?;
        
        if num < 1 || num > 12 {
            return None;
        }
        
        // Circle of fifths mapping
        let circle_of_fifths_major = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5]; // C, G, D, A, E, B, F#, C#, G#, D#, A#, F
        let circle_of_fifths_minor = [9, 4, 11, 6, 1, 8, 3, 10, 5, 0, 7, 2]; // Am, Em, Bm, F#m, C#m, G#m, D#m, A#m, Fm, Cm, Gm, Dm
        
        match suffix {
            "A" => {
                let key_idx = circle_of_fifths_major[(num - 1) as usize];
                Some(Key::Major(key_idx))
            }
            "B" => {
                let key_idx = circle_of_fifths_minor[(num - 1) as usize];
                Some(Key::Minor(key_idx))
            }
            _ => None,
        }
    }
}

/// Beat grid structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatGrid {
    /// Downbeat times (beat 1) in seconds
    pub downbeats: Vec<f32>,
    
    /// All beat times in seconds
    pub beats: Vec<f32>,
    
    /// Bar boundaries in seconds
    pub bars: Vec<f32>,
}

/// Analysis flags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalysisFlag {
    /// Multiple BPM peaks equally strong
    MultimodalBpm,
    /// Low key clarity (atonal/ambiguous)
    WeakTonality,
    /// Track has tempo drift
    TempoVariation,
    /// Multiple onset interpretations
    OnsetDetectionAmbiguous,
}

/// Tempogram candidate diagnostics (for validation/tuning)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoCandidateDebug {
    /// Candidate tempo in BPM.
    pub bpm: f32,
    /// Combined score used for ranking (blended FFT+autocorr, with mild priors).
    pub score: f32,
    /// FFT-method normalized support in [0, 1].
    pub fft_norm: f32,
    /// Autocorrelation-method normalized support in [0, 1].
    pub autocorr_norm: f32,
    /// True if this candidate was selected as the final BPM.
    pub selected: bool,
}

/// Complete analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// BPM estimate
    pub bpm: f32,
    
    /// BPM confidence (0.0-1.0)
    pub bpm_confidence: f32,
    
    /// Detected key
    pub key: Key,
    
    /// Key confidence (0.0-1.0)
    pub key_confidence: f32,
    
    /// Key clarity (0.0-1.0)
    /// 
    /// Measures how "tonal" vs "atonal" the track is:
    /// - High (>0.5): Strong tonality, reliable key detection
    /// - Medium (0.2-0.5): Moderate tonality
    /// - Low (<0.2): Weak tonality, key detection may be unreliable
    pub key_clarity: f32,
    
    /// Beat grid
    pub beat_grid: BeatGrid,
    
    /// Grid stability (0.0-1.0)
    pub grid_stability: f32,
    
    /// Analysis metadata
    pub metadata: AnalysisMetadata,
}

/// Analysis metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisMetadata {
    /// Audio duration in seconds
    pub duration_seconds: f32,
    
    /// Sample rate in Hz
    pub sample_rate: u32,
    
    /// Processing time in milliseconds
    pub processing_time_ms: f32,
    
    /// Algorithm version
    pub algorithm_version: String,
    
    /// Onset method consensus score
    pub onset_method_consensus: f32,
    
    /// Methods used
    pub methods_used: Vec<String>,
    
    /// Analysis flags
    pub flags: Vec<AnalysisFlag>,
    
    /// Confidence warnings (low confidence, ambiguous results, etc.)
    pub confidence_warnings: Vec<String>,

    /// Optional: tempogram candidate list (top-N) for diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_candidates: Option<Vec<TempoCandidateDebug>>,

    /// Tempogram multi-resolution escalation was triggered for this track (ambiguous base estimate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_multi_res_triggered: Option<bool>,

    /// Tempogram multi-resolution result was selected over the base single-resolution estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_multi_res_used: Option<bool>,

    /// Tempogram percussive-only fallback was triggered (ambiguous estimate + HPSS enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_percussive_triggered: Option<bool>,

    /// Tempogram percussive-only fallback was selected over the current estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_percussive_used: Option<bool>,
}

// Re-export for convenience
pub use Key as KeyType;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_key_name_major() {
        assert_eq!(Key::Major(0).name(), "C");
        assert_eq!(Key::Major(1).name(), "C#");
        assert_eq!(Key::Major(2).name(), "D");
        assert_eq!(Key::Major(6).name(), "F#");
        assert_eq!(Key::Major(11).name(), "B");
    }
    
    #[test]
    fn test_key_name_minor() {
        assert_eq!(Key::Minor(0).name(), "Cm");
        assert_eq!(Key::Minor(1).name(), "C#m");
        assert_eq!(Key::Minor(2).name(), "Dm");
        assert_eq!(Key::Minor(9).name(), "Am");
        assert_eq!(Key::Minor(11).name(), "Bm");
    }
    
    #[test]
    fn test_key_numerical_major() {
        // Circle of fifths: C=1A, G=2A, D=3A, A=4A, E=5A, B=6A, F#=7A, C#=8A, G#=9A, D#=10A, A#=11A, F=12A
        assert_eq!(Key::Major(0).numerical(), "1A");   // C
        assert_eq!(Key::Major(7).numerical(), "2A");   // G
        assert_eq!(Key::Major(2).numerical(), "3A");   // D
        assert_eq!(Key::Major(9).numerical(), "4A");   // A
        assert_eq!(Key::Major(4).numerical(), "5A");   // E
        assert_eq!(Key::Major(11).numerical(), "6A");  // B
        assert_eq!(Key::Major(6).numerical(), "7A");   // F#
        assert_eq!(Key::Major(1).numerical(), "8A");   // C#
        assert_eq!(Key::Major(8).numerical(), "9A");   // G#
        assert_eq!(Key::Major(3).numerical(), "10A");  // D#
        assert_eq!(Key::Major(10).numerical(), "11A"); // A#
        assert_eq!(Key::Major(5).numerical(), "12A"); // F
    }
    
    #[test]
    fn test_key_numerical_minor() {
        // Circle of fifths: Am=1B, Em=2B, Bm=3B, F#m=4B, C#m=5B, G#m=6B, D#m=7B, A#m=8B, Fm=9B, Cm=10B, Gm=11B, Dm=12B
        assert_eq!(Key::Minor(9).numerical(), "1B");   // Am
        assert_eq!(Key::Minor(4).numerical(), "2B");   // Em
        assert_eq!(Key::Minor(11).numerical(), "3B");  // Bm
        assert_eq!(Key::Minor(6).numerical(), "4B");   // F#m
        assert_eq!(Key::Minor(1).numerical(), "5B");   // C#m
        assert_eq!(Key::Minor(8).numerical(), "6B");   // G#m
        assert_eq!(Key::Minor(3).numerical(), "7B");   // D#m
        assert_eq!(Key::Minor(10).numerical(), "8B");  // A#m
        assert_eq!(Key::Minor(5).numerical(), "9B");   // Fm
        assert_eq!(Key::Minor(0).numerical(), "10B");  // Cm
        assert_eq!(Key::Minor(7).numerical(), "11B");  // Gm
        assert_eq!(Key::Minor(2).numerical(), "12B");  // Dm
    }
    
    #[test]
    fn test_key_from_numerical() {
        // Test major keys
        assert_eq!(Key::from_numerical("1A"), Some(Key::Major(0)));   // C
        assert_eq!(Key::from_numerical("2A"), Some(Key::Major(7)));   // G
        assert_eq!(Key::from_numerical("7A"), Some(Key::Major(6)));   // F#
        assert_eq!(Key::from_numerical("12A"), Some(Key::Major(5)));  // F
        
        // Test minor keys
        assert_eq!(Key::from_numerical("1B"), Some(Key::Minor(9)));   // Am
        assert_eq!(Key::from_numerical("2B"), Some(Key::Minor(4)));   // Em
        assert_eq!(Key::from_numerical("10B"), Some(Key::Minor(0)));  // Cm
        
        // Test invalid inputs
        assert_eq!(Key::from_numerical("0A"), None);
        assert_eq!(Key::from_numerical("13A"), None);
        assert_eq!(Key::from_numerical("1C"), None);
        assert_eq!(Key::from_numerical(""), None);
        assert_eq!(Key::from_numerical("A"), None);
    }
    
    #[test]
    fn test_key_numerical_roundtrip() {
        // Test that numerical() and from_numerical() are inverses
        for i in 0..12 {
            let major = Key::Major(i);
            let num = major.numerical();
            assert_eq!(Key::from_numerical(&num), Some(major), "Failed roundtrip for major key {}: {}", i, num);
            
            let minor = Key::Minor(i);
            let num = minor.numerical();
            assert_eq!(Key::from_numerical(&num), Some(minor), "Failed roundtrip for minor key {}: {}", i, num);
        }
    }
}

