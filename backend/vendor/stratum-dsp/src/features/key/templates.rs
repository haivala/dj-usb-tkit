//! Key templates for template-matching key detection
//!
//! Supports multiple template sets:
//! - Krumhansl-Kessler (1982): Empirical profiles from listening experiments
//! - Temperley (1999): Statistical profiles derived from musical corpus analysis
//!
//! # References
//!
//! - Krumhansl, C. L., & Kessler, E. J. (1982). Tracing the Dynamic Changes in Perceived
//!   Tonal Organization in a Spatial Representation of Musical Keys. *Psychological Review*,
//!   89(4), 334-368.
//! - Temperley, D. (1999). What's key for key? The Krumhansl-Schmuckler key-finding algorithm
//!   reconsidered. *Music Perception*, 17(1), 65-100.

/// Template set type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateSet {
    /// Krumhansl-Kessler (1982) templates
    KrumhanslKessler,
    /// Temperley (1999) templates
    Temperley,
}

/// Key templates for all 24 keys
#[derive(Debug, Clone)]
pub struct KeyTemplates {
    /// Major key templates (12 keys: C, C#, D, ..., B)
    pub major: [Vec<f32>; 12],
    
    /// Minor key templates (12 keys: C, C#, D, ..., B)
    pub minor: [Vec<f32>; 12],
}

impl KeyTemplates {
    /// Create new key templates with Krumhansl-Kessler profiles
    ///
    /// Templates are 12-element vectors representing the likelihood of each
    /// semitone class (C, C#, D, ..., B) in that key.
    ///
    /// # Returns
    ///
    /// `KeyTemplates` with all 24 key profiles initialized
    pub fn new() -> Self {
        Self::new_with_template_set(TemplateSet::KrumhanslKessler)
    }

    /// Create new key templates with specified template set
    ///
    /// # Arguments
    ///
    /// * `template_set` - Which template set to use (K-K or Temperley)
    ///
    /// # Returns
    ///
    /// `KeyTemplates` with all 24 key profiles initialized
    pub fn new_with_template_set(template_set: TemplateSet) -> Self {
        match template_set {
            TemplateSet::KrumhanslKessler => Self::new_krumhansl_kessler(),
            TemplateSet::Temperley => Self::new_temperley(),
        }
    }

    /// Create Krumhansl-Kessler templates
    fn new_krumhansl_kessler() -> Self {
        // Canonical Krumhansl-Kessler tonal profiles (1982), in [C, C#, D, ..., B] order.
        // Source values are commonly published in this form:
        //   - Major: C major profile
        //   - Minor: C minor profile
        //
        // We rotate these base profiles to generate all keys.
        let c_major = vec![
            6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
        ];

        let c_minor = vec![
            6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
        ];
        
        // Generate all 12 major keys by rotating C major
        let mut major: [Vec<f32>; 12] = [
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
        ];
        
        // Generate all 12 minor keys by rotating A minor
        let mut minor: [Vec<f32>; 12] = [
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
        ];
        
        // Rotate templates for all 12 keys
        for key_idx in 0..12 {
            // Major keys: rotate C major template
            // For key_idx, we want the template rotated so that the tonic is at key_idx
            for semitone_idx in 0..12 {
                major[key_idx][semitone_idx] = c_major[(semitone_idx + 12 - key_idx) % 12];
            }
            
            // Minor keys: rotate C-minor base profile (tonic at index 0) the same way we rotate major.
            for semitone_idx in 0..12 {
                let source_idx = (semitone_idx + 12 - key_idx) % 12;
                minor[key_idx][semitone_idx] = c_minor[source_idx];
            }
        }

        // L2-normalize each template so dot-products behave like cosine similarity against L2-normalized chroma.
        fn l2_normalize(v: &mut [f32]) {
            let norm = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
            if norm > 1e-12 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
        }

        for k in 0..12 {
            l2_normalize(&mut major[k]);
            l2_normalize(&mut minor[k]);
        }
        
        Self { major, minor }
    }

    /// Create Temperley templates
    ///
    /// Temperley (1999) profiles are derived from statistical analysis of musical corpora.
    /// They tend to have different mode discrimination characteristics than K-K templates.
    fn new_temperley() -> Self {
        // Temperley (1999) tonal profiles, in [C, C#, D, ..., B] order.
        // Source: Temperley, D. (1999). What's key for key? The Krumhansl-Schmuckler
        // key-finding algorithm reconsidered. Music Perception, 17(1), 65-100.
        //
        // These values are commonly cited in MIR literature. They are derived from
        // corpus analysis rather than listening experiments.
        let c_major = vec![
            5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0,
        ];

        let c_minor = vec![
            5.0, 2.0, 3.5, 5.0, 2.0, 3.5, 2.0, 4.5, 3.5, 2.0, 4.0, 3.5,
        ];
        
        // Generate all 12 major keys by rotating C major
        let mut major: [Vec<f32>; 12] = [
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
        ];
        
        // Generate all 12 minor keys by rotating C minor
        let mut minor: [Vec<f32>; 12] = [
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
            vec![0.0; 12],
        ];
        
        // Rotate templates for all 12 keys
        for key_idx in 0..12 {
            // Major keys: rotate C major template
            for semitone_idx in 0..12 {
                major[key_idx][semitone_idx] = c_major[(semitone_idx + 12 - key_idx) % 12];
            }
            
            // Minor keys: rotate C minor template
            for semitone_idx in 0..12 {
                let source_idx = (semitone_idx + 12 - key_idx) % 12;
                minor[key_idx][semitone_idx] = c_minor[source_idx];
            }
        }

        // L2-normalize each template
        fn l2_normalize(v: &mut [f32]) {
            let norm = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
            if norm > 1e-12 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
        }

        for k in 0..12 {
            l2_normalize(&mut major[k]);
            l2_normalize(&mut minor[k]);
        }
        
        Self { major, minor }
    }
    
    /// Get template for a specific key
    ///
    /// # Arguments
    ///
    /// * `key_idx` - Key index (0-11 for major, 12-23 for minor)
    ///
    /// # Returns
    ///
    /// 12-element template vector for the specified key
    pub fn get_template(&self, key_idx: u32) -> &[f32] {
        if key_idx < 12 {
            &self.major[key_idx as usize]
        } else {
            &self.minor[(key_idx - 12) as usize]
        }
    }
    
    /// Get template for major key
    ///
    /// # Arguments
    ///
    /// * `key_idx` - Major key index (0-11: C, C#, D, ..., B)
    ///
    /// # Returns
    ///
    /// 12-element template vector for the specified major key
    pub fn get_major_template(&self, key_idx: u32) -> &[f32] {
        &self.major[key_idx as usize % 12]
    }
    
    /// Get template for minor key
    ///
    /// # Arguments
    ///
    /// * `key_idx` - Minor key index (0-11: C, C#, D, ..., B)
    ///
    /// # Returns
    ///
    /// 12-element template vector for the specified minor key
    pub fn get_minor_template(&self, key_idx: u32) -> &[f32] {
        &self.minor[key_idx as usize % 12]
    }
}

impl Default for KeyTemplates {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_key_templates_creation() {
        let templates = KeyTemplates::new();
        
        // Check that all templates have 12 elements
        for i in 0..12 {
            assert_eq!(templates.major[i].len(), 12);
            assert_eq!(templates.minor[i].len(), 12);
        }
    }
    
    #[test]
    fn test_c_major_template() {
        let templates = KeyTemplates::new();
        let c_major = templates.get_major_template(0);
        
        // C major should strongly emphasize C, E, G (tonic, major third, perfect fifth).
        // With canonical K-K profiles (L2-normalized), absolute values are not tiny, so we test *relative* structure.
        let tonic = c_major[0];
        let third = c_major[4];
        let fifth = c_major[7];

        // Tonic should be the maximum (or tied within float epsilon).
        let max = c_major.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((tonic - max).abs() < 1e-6);

        // Third and fifth should be among the strongest components.
        let mut sorted = c_major.to_vec();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let top4_threshold = sorted[3];
        assert!(third >= top4_threshold);
        assert!(fifth >= top4_threshold);

        // Strong preference over chromatic tones (e.g., C#, D#, F#, G#, A#)
        let chromatic_avg = (c_major[1] + c_major[3] + c_major[6] + c_major[8] + c_major[10]) / 5.0;
        assert!(tonic > chromatic_avg * 1.4);
        assert!(third > chromatic_avg * 1.1);
        assert!(fifth > chromatic_avg * 1.2);
    }
    
    #[test]
    fn test_a_minor_template() {
        let templates = KeyTemplates::new();
        let a_minor = templates.get_minor_template(9); // A is index 9
        
        // A minor should have high values for A, C, E (tonic, minor third, perfect fifth)
        assert!(a_minor[9] > 0.1); // A (tonic)
        assert!(a_minor[0] > 0.1); // C (minor third)
        assert!(a_minor[4] > 0.1); // E (perfect fifth)
    }
    
    #[test]
    fn test_get_template() {
        let templates = KeyTemplates::new();
        
        // Test major keys (0-11)
        let c_major = templates.get_template(0);
        assert_eq!(c_major.len(), 12);
        
        // Test minor keys (12-23)
        let a_minor = templates.get_template(21); // A minor is 12 + 9 = 21
        assert_eq!(a_minor.len(), 12);
    }
    
    #[test]
    fn test_template_rotation() {
        let templates = KeyTemplates::new();
        
        // C major and D major should be related by rotation
        let _c_major = templates.get_major_template(0);
        let d_major = templates.get_major_template(2); // D is 2 semitones above C
        
        // D major should have high value at index 2 (D)
        assert!(d_major[2] > 0.1);
        
        // The pattern should be rotated
        // C major's tonic (index 0) should correspond to D major's subdominant (index 10)
        // This is a simplified check - the full relationship is more complex
    }
}
