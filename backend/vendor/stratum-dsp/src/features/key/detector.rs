//! Key detection algorithm
//!
//! Matches chroma distribution against Krumhansl-Kessler templates to detect
//! the musical key of an audio track.
//!
//! # Reference
//!
//! Krumhansl, C. L., & Kessler, E. J. (1982). Tracing the Dynamic Changes in Perceived
//! Tonal Organization in a Spatial Representation of Musical Keys. *Psychological Review*,
//! 89(4), 334-368.

use crate::error::AnalysisError;
use crate::analysis::result::Key;
use super::{templates::{KeyTemplates, TemplateSet}, KeyDetectionResult, compute_key_clarity};

/// Detect musical key from chroma vectors
///
/// Averages chroma vectors across all frames, then computes dot product with
/// each of the 24 key templates. The key with the highest score is selected.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `templates` - Key templates (Krumhansl-Kessler profiles)
///
/// # Returns
///
/// Key detection result with:
/// - Detected key (major or minor, 0-11)
/// - Confidence score (0.0-1.0)
/// - All 24 key scores (ranked)
///
/// # Errors
///
/// Returns `AnalysisError` if:
/// - Chroma vectors are empty
/// - Chroma vectors have incorrect dimensions
///
/// # Example
///
/// ```no_run
/// use stratum_dsp::features::key::{detector::detect_key, templates::KeyTemplates};
/// use stratum_dsp::features::chroma::extractor::extract_chroma;
///
/// let samples = vec![0.0f32; 44100 * 5];
/// let chroma_vectors = extract_chroma(&samples, 44100, 2048, 512)?;
/// let templates = KeyTemplates::new();
/// let result = detect_key(&chroma_vectors, &templates)?;
///
/// println!("Detected key: {:?}, confidence: {:.2}", result.key, result.confidence);
/// # Ok::<(), stratum_dsp::AnalysisError>(())
/// ```
pub fn detect_key(
    chroma_vectors: &[Vec<f32>],
    templates: &KeyTemplates,
) -> Result<KeyDetectionResult, AnalysisError> {
    detect_key_weighted(chroma_vectors, templates, None)
}

/// Detect musical key from chroma vectors with optional per-frame weights.
///
/// This is a small but important upgrade over simple averaging: weighting allows the caller to
/// emphasize frames with stronger/cleaner tonality (e.g., suppress percussive/noisy frames).
pub fn detect_key_weighted(
    chroma_vectors: &[Vec<f32>],
    templates: &KeyTemplates,
    frame_weights: Option<&[f32]>,
) -> Result<KeyDetectionResult, AnalysisError> {
    log::debug!(
        "Detecting key from {} chroma vectors (weighted={})",
        chroma_vectors.len(),
        frame_weights.is_some()
    );
    
    if chroma_vectors.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Empty chroma vectors".to_string()
        ));
    }
    
    // Validate chroma vector dimensions
    let n_semitones = chroma_vectors[0].len();
    if n_semitones != 12 {
        return Err(AnalysisError::InvalidInput(format!(
            "Chroma vectors must have 12 elements, got {}",
            n_semitones
        )));
    }
    
    for (i, chroma) in chroma_vectors.iter().enumerate() {
        if chroma.len() != 12 {
            return Err(AnalysisError::InvalidInput(format!(
                "Chroma vector at index {} has {} elements, expected 12",
                i, chroma.len()
            )));
        }
    }
    
    if let Some(w) = frame_weights {
        if w.len() != chroma_vectors.len() {
            return Err(AnalysisError::InvalidInput(format!(
                "frame_weights length mismatch: got {}, expected {}",
                w.len(),
                chroma_vectors.len()
            )));
        }
    }

    // Step 1: Compute weighted scores for all 24 keys.
    //
    // We keep the scoring frame-based (sum of dot products) so we can apply frame weights
    // without constructing a potentially biased global histogram.
    let mut scores = Vec::with_capacity(24);
    let weights = frame_weights;

    // Major keys (0-11)
    for key_idx in 0..12 {
        let template = templates.get_major_template(key_idx);
        let score = weighted_sum_dot(chroma_vectors, weights, template);
        scores.push((Key::Major(key_idx), score));
    }

    // Minor keys (0-11)
    for key_idx in 0..12 {
        let template = templates.get_minor_template(key_idx);
        let score = weighted_sum_dot(chroma_vectors, weights, template);
        scores.push((Key::Minor(key_idx), score));
    }
    
    // Step 1.5: Normalize major and minor scores separately to address scale differences.
    // This helps when major and minor template scores have different ranges, which can
    // bias mode selection. We normalize by the max score in each mode category.
    let max_major = scores.iter()
        .filter_map(|(k, s)| if matches!(k, Key::Major(_)) { Some(*s) } else { None })
        .fold(0.0f32, f32::max);
    let max_minor = scores.iter()
        .filter_map(|(k, s)| if matches!(k, Key::Minor(_)) { Some(*s) } else { None })
        .fold(0.0f32, f32::max);
    
    // Normalize scores if max values are non-zero
    if max_major > 1e-9 && max_minor > 1e-9 {
        for (k, s) in scores.iter_mut() {
            match k {
                Key::Major(_) => *s /= max_major,
                Key::Minor(_) => *s /= max_minor,
            }
        }
    }
    
    // Step 1.75: Circle-of-fifths distance weighting (optional refinement)
    // Keys close on the circle of fifths (e.g., C-G, C-F) are harmonically related
    // and often confused. We can boost scores of keys that are close to high-scoring keys.
    // This helps when the true key is a neighbor on the circle of fifths.
    let circle_of_fifths = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5]; // C, G, D, A, E, B, F#, C#, G#, D#, A#, F
    let mut refined_scores = scores.clone();
    
    // Find the top-scoring key for each mode
    let (top_major_key, top_major_score) = scores.iter()
        .filter_map(|(k, s)| if matches!(k, Key::Major(_)) { Some((k, s)) } else { None })
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((&Key::Major(0), &0.0));
    let (top_minor_key, top_minor_score) = scores.iter()
        .filter_map(|(k, s)| if matches!(k, Key::Minor(_)) { Some((k, s)) } else { None })
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((&Key::Minor(0), &0.0));
    
    // Apply circle-of-fifths bonus to keys near the top-scoring keys
    let circle_bonus_weight = 0.20; // 20% bonus for adjacent keys (increased from 15%)
    for (k, s) in refined_scores.iter_mut() {
        let (ref_key, ref_score) = match k {
            Key::Major(_) => (top_major_key, top_major_score),
            Key::Minor(_) => (top_minor_key, top_minor_score),
        };
        
        if *ref_score > 1e-9 {
            let tonic = match k {
                Key::Major(i) => *i as usize,
                Key::Minor(i) => *i as usize,
            };
            let ref_tonic = match ref_key {
                Key::Major(i) => *i as usize,
                Key::Minor(i) => *i as usize,
            };
            
            // Find positions on circle of fifths
            let tonic_pos = circle_of_fifths.iter().position(|&x| x == tonic).unwrap_or(12);
            let ref_pos = circle_of_fifths.iter().position(|&x| x == ref_tonic).unwrap_or(12);
            
            if tonic_pos < 12 && ref_pos < 12 {
                // Compute circular distance (min of forward and backward)
                let dist = (tonic_pos as i32 - ref_pos as i32).abs().min(12 - (tonic_pos as i32 - ref_pos as i32).abs());
                
                // Apply bonus: adjacent keys (dist=1) get full bonus, distance 2 gets half bonus
                if dist <= 2 {
                    let bonus = circle_bonus_weight * (1.0 - (dist as f32) * 0.5);
                    *s += *ref_score * bonus;
                }
            }
        }
    }
    
    scores = refined_scores;
    
    // Step 2: Sort by score (highest first)
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    
    // Step 3: Select best key using weighted top-N voting (if top keys are close)
    // This helps when the best key is only slightly better than alternatives
    let (best_key, best_score) = scores[0];
    let second_score = if scores.len() > 1 { scores[1].1 } else { 0.0 };
    let third_score = if scores.len() > 2 { scores[2].1 } else { 0.0 };
    
    // If top 3 keys are within 5% of each other, use weighted voting
    let score_threshold = best_score * 0.95;
    let use_weighted_voting = second_score >= score_threshold && third_score >= score_threshold * 0.90;
    
    let final_key = if use_weighted_voting {
        // Weighted voting: count occurrences of each key in top 3, weighted by score
        let mut key_votes: std::collections::HashMap<Key, f32> = std::collections::HashMap::new();
        for (key, score) in scores.iter().take(3) {
            let vote_weight = *score / best_score; // Normalize by best score
            *key_votes.entry(*key).or_insert(0.0) += vote_weight;
        }
        
        // Select key with highest vote count
        key_votes.iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| *k)
            .unwrap_or(best_key)
    } else {
        best_key
    };
    
    // Compute confidence for final key
    let final_score = scores.iter()
        .find(|(k, _)| *k == final_key)
        .map(|(_, s)| *s)
        .unwrap_or(best_score);
    let best_other = scores.iter()
        .find(|(k, _)| *k != final_key)
        .map(|(_, s)| *s)
        .unwrap_or(0.0);
    
    let confidence = if final_score > 0.0 {
        ((final_score - best_other) / final_score).max(0.0).min(1.0)
    } else {
        0.0
    };
    
    // Step 4: Extract top N keys (default: top 3)
    let top_n = 3;
    let top_keys: Vec<(Key, f32)> = scores.iter()
        .take(top_n)
        .cloned()
        .collect();
    
    log::debug!("Detected key: {:?}, score: {:.4}, confidence: {:.4} (weighted voting: {})",
                final_key, final_score, confidence, use_weighted_voting);
    
    Ok(KeyDetectionResult {
        key: final_key,
        confidence,
        all_scores: scores,
        top_keys,
    })
}

/// Detect musical key with an additional (optional) mode heuristic to reduce minor→major mistakes.
///
/// Motivation:
/// - On real-world DJ mixes, template scores often identify the tonic correctly but choose the wrong mode
///   (especially minor keys predicted as major). A lightweight heuristic using the 3rd scale degree can help.
///
/// Heuristic:
/// - If the best-scoring key is Major(tonic) but the chroma energy at (tonic+3) (minor 3rd) clearly exceeds
///   (tonic+4) (major 3rd), we may flip to Minor(tonic), but only if the Minor(tonic) score is close enough.
///
/// This is intentionally conservative and gated by a score ratio threshold to avoid harming major tracks.
pub fn detect_key_weighted_mode_heuristic(
    chroma_vectors: &[Vec<f32>],
    templates: &KeyTemplates,
    frame_weights: Option<&[f32]>,
    third_ratio_margin: f32,
    flip_min_score_ratio: f32,
    enable_minor_harmonic_bonus: bool,
    minor_leading_tone_bonus_weight: f32,
) -> Result<KeyDetectionResult, AnalysisError> {
    // First, compute the standard template scores.
    let base = detect_key_weighted(chroma_vectors, templates, frame_weights)?;

    let flip_ratio = flip_min_score_ratio.clamp(0.0, 1.0);
    let enable_mode_flip = flip_ratio > 0.0;
    if !enable_minor_harmonic_bonus && !enable_mode_flip {
        return Ok(base);
    }

    // Compute a simple weighted average chroma distribution for the 3rd-degree test.
    let mut avg = vec![0.0f32; 12];
    let mut wsum = 0.0f32;
    match frame_weights {
        None => {
            for ch in chroma_vectors {
                for i in 0..12 {
                    avg[i] += ch[i];
                }
            }
            wsum = chroma_vectors.len() as f32;
        }
        Some(w) => {
            for (ch, &wt) in chroma_vectors.iter().zip(w.iter()) {
                if wt <= 0.0 {
                    continue;
                }
                for i in 0..12 {
                    avg[i] += wt * ch[i];
                }
                wsum += wt;
            }
        }
    }

    if wsum <= 1e-9 {
        return Ok(base);
    }
    let sum: f32 = avg.iter().sum();
    if sum > 1e-9 {
        for x in avg.iter_mut() {
            *x /= sum;
        }
    }

    // Optional: apply a minor-key harmonic bonus (leading-tone vs flat-7) to the score table.
    // We do this *after* base scoring so we can keep the existing template code unchanged.
    let mut scores = base.all_scores.clone();
    if enable_minor_harmonic_bonus {
        let w = minor_leading_tone_bonus_weight.max(0.0);
        if w > 0.0 {
            for (k, s) in scores.iter_mut() {
                if let Key::Minor(tonic_u32) = *k {
                    let tonic = tonic_u32 as usize;
                    let lt = (tonic + 11) % 12; // leading tone
                    let b7 = (tonic + 10) % 12; // flat-7
                    *s += wsum * w * (avg[lt] - avg[b7]);
                }
            }
        }
    }

    // Re-sort with the bonus applied and compute the new best key.
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let (best_key, _best_score) = scores[0];

    // Build quick score lookup tables for parallel/tonic mode flipping (post-bonus).
    let mut major_scores = [0.0f32; 12];
    let mut minor_scores = [0.0f32; 12];
    for (k, s) in scores.iter() {
        match *k {
            Key::Major(i) => major_scores[i as usize] = *s,
            Key::Minor(i) => minor_scores[i as usize] = *s,
        }
    }
    let (tonic, best_is_major) = match best_key {
        Key::Major(i) => (i as usize, true),
        Key::Minor(i) => (i as usize, false),
    };

    // Enhanced mode discrimination: use weighted combination of all distinguishing scale degrees
    // Major and minor scales differ in: 3rd (3 vs 4 semitones), 6th (8 vs 9), 7th (10 vs 11)
    // We compute a weighted score favoring the mode that matches the chroma distribution better
    let p_min3 = avg[(tonic + 3) % 12];
    let p_maj3 = avg[(tonic + 4) % 12];
    let p_min6 = avg[(tonic + 8) % 12];  // minor 6th (8 semitones)
    let p_maj6 = avg[(tonic + 9) % 12];  // major 6th (9 semitones)
    let p_min7 = avg[(tonic + 10) % 12]; // minor 7th (flat-7, 10 semitones)
    let p_maj7 = avg[(tonic + 11) % 12]; // major 7th (leading tone, 11 semitones)
    
    let margin = third_ratio_margin.max(0.0);
    
    // Compute weighted mode scores: each distinguishing degree contributes to the mode score
    // Weight by the strength of the difference (how much one exceeds the other)
    let mut minor_score = 0.0f32;
    let mut major_score = 0.0f32;
    
    // 3rd degree (most important for mode discrimination)
    let third_diff = (p_min3 - p_maj3).abs();
    if p_min3 > (p_maj3 * (1.0 + margin)) {
        minor_score += third_diff * 2.0; // Weight 3rd degree heavily
    } else if p_maj3 > (p_min3 * (1.0 + margin)) {
        major_score += third_diff * 2.0;
    }
    
    // 6th degree
    let sixth_diff = (p_min6 - p_maj6).abs();
    if p_min6 > (p_maj6 * (1.0 + margin)) {
        minor_score += sixth_diff * 1.0;
    } else if p_maj6 > (p_min6 * (1.0 + margin)) {
        major_score += sixth_diff * 1.0;
    }
    
    // 7th degree
    let seventh_diff = (p_min7 - p_maj7).abs();
    if p_min7 > (p_maj7 * (1.0 + margin)) {
        minor_score += seventh_diff * 1.0;
    } else if p_maj7 > (p_min7 * (1.0 + margin)) {
        major_score += seventh_diff * 1.0;
    }
    
    // Mode preference: use weighted scores instead of simple votes
    let total_mode_score = minor_score + major_score;
    let minor_pref = if total_mode_score > 1e-9 {
        minor_score > major_score * (1.0 + margin * 0.5)
    } else {
        false
    };
    let major_pref = if total_mode_score > 1e-9 {
        major_score > minor_score * (1.0 + margin * 0.5)
    } else {
        false
    };

    let mut chosen_key = best_key;
    if enable_mode_flip {
        if best_is_major && minor_pref {
            let s_best = major_scores[tonic];
            let s_alt = minor_scores[tonic];
            if s_best > 0.0 && s_alt >= s_best * flip_ratio {
                chosen_key = Key::Minor(tonic as u32);
            }
        } else if !best_is_major && major_pref {
            let s_best = minor_scores[tonic];
            let s_alt = major_scores[tonic];
            if s_best > 0.0 && s_alt >= s_best * flip_ratio {
                chosen_key = Key::Major(tonic as u32);
            }
        }
    }

    // Recompute confidence for the chosen key (may be lower than the best score if we flip mode).
    let chosen_score = match chosen_key {
        Key::Major(i) => major_scores[i as usize],
        Key::Minor(i) => minor_scores[i as usize],
    };

    let mut best_other = 0.0f32;
    for (k, s) in scores.iter() {
        if *k == chosen_key {
            continue;
        }
        best_other = best_other.max(*s);
    }

    let confidence = if chosen_score > 0.0 {
        ((chosen_score - best_other) / chosen_score).max(0.0).min(1.0)
    } else {
        0.0
    };

    // Top-N from the post-bonus score table.
    let mut top_keys = scores.iter().take(3).cloned().collect::<Vec<_>>();
    if !top_keys.iter().any(|(k, _)| *k == chosen_key) {
        top_keys.pop();
        top_keys.push((chosen_key, chosen_score));
    }

    Ok(KeyDetectionResult {
        key: chosen_key,
        confidence,
        all_scores: scores,
        top_keys,
    })
}

/// Multi-scale key detection: run key detection at multiple time scales and aggregate results.
///
/// This ensemble approach runs key detection at multiple segment lengths (short, medium, long)
/// and aggregates the results using clarity-weighted voting. This captures both local and
/// global key information, improving robustness on tracks with key changes or varying harmonic
/// stability.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `templates` - Key templates (Krumhansl-Kessler profiles)
/// * `frame_weights` - Optional per-frame weights
/// * `segment_lengths_frames` - Vector of segment lengths to use (in frames)
/// * `segment_hop_frames` - Hop size between segments (in frames)
/// * `min_clarity` - Minimum clarity threshold for including a segment
/// * `scale_weights` - Optional weights for each scale (if None, all scales weighted equally)
/// * `enable_mode_heuristic` - Enable mode heuristic (3rd-degree test)
/// * `mode_third_ratio_margin` - Margin for 3rd-degree ratio test
/// * `mode_flip_min_score_ratio` - Minimum score ratio for mode flip
/// * `enable_minor_harmonic_bonus` - Enable minor harmonic bonus
/// * `minor_leading_tone_bonus_weight` - Weight for minor leading-tone bonus
///
/// # Returns
///
/// Key detection result aggregated across all scales
pub fn detect_key_multi_scale(
    chroma_vectors: &[Vec<f32>],
    templates: &KeyTemplates,
    frame_weights: Option<&[f32]>,
    segment_lengths_frames: &[usize],
    segment_hop_frames: usize,
    min_clarity: f32,
    scale_weights: Option<&[f32]>,
    enable_mode_heuristic: bool,
    mode_third_ratio_margin: f32,
    mode_flip_min_score_ratio: f32,
    enable_minor_harmonic_bonus: bool,
    minor_leading_tone_bonus_weight: f32,
) -> Result<KeyDetectionResult, AnalysisError> {
    if chroma_vectors.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Empty chroma vectors".to_string()
        ));
    }
    
    if segment_lengths_frames.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "No segment lengths provided for multi-scale detection".to_string()
        ));
    }
    
    // Initialize accumulated score table (24 keys)
    let mut acc_scores: Vec<(Key, f32)> = Vec::with_capacity(24);
    for k in 0..12 {
        acc_scores.push((Key::Major(k as u32), 0.0));
    }
    for k in 0..12 {
        acc_scores.push((Key::Minor(k as u32), 0.0));
    }
    
    let mut total_weight = 0.0f32;
    let mut used_segments = 0usize;
    
    // Process each scale
    for (scale_idx, &seg_len) in segment_lengths_frames.iter().enumerate() {
        if seg_len == 0 || seg_len > chroma_vectors.len() {
            continue;
        }
        
        let scale_weight = scale_weights
            .and_then(|w| w.get(scale_idx).copied())
            .unwrap_or(1.0);
        
        if scale_weight <= 0.0 {
            continue;
        }
        
        let hop = segment_hop_frames.max(1);
        let mut start = 0usize;
        
        // Process all segments at this scale
        while start + seg_len <= chroma_vectors.len() {
            let seg = &chroma_vectors[start..start + seg_len];
            let wseg = frame_weights
                .and_then(|w| {
                    if start + seg_len <= w.len() {
                        Some(&w[start..start + seg_len])
                    } else {
                        None
                    }
                });
            
            // Detect key for this segment
            let seg_res = if enable_mode_heuristic || enable_minor_harmonic_bonus {
                detect_key_weighted_mode_heuristic(
                    seg,
                    templates,
                    wseg,
                    mode_third_ratio_margin,
                    if enable_mode_heuristic {
                        mode_flip_min_score_ratio
                    } else {
                        0.0
                    },
                    enable_minor_harmonic_bonus,
                    minor_leading_tone_bonus_weight,
                )?
            } else {
                detect_key_weighted(seg, templates, wseg)?
            };
            
            let seg_clarity = compute_key_clarity(&seg_res.all_scores);
            
            // Only include segments above clarity threshold
            if seg_clarity >= min_clarity {
                used_segments += 1;
                let combined_weight = seg_clarity * scale_weight;
                total_weight += combined_weight;
                
                // Accumulate scores weighted by clarity and scale weight
                for (k, s) in seg_res.all_scores.iter() {
                    if let Some((_kk, dst)) = acc_scores.iter_mut().find(|(kk, _)| kk == k) {
                        *dst += *s * combined_weight;
                    }
                }
            }
            
            start += hop;
        }
    }
    
    // If no segments were used, fall back to full-track detection
    if used_segments == 0 || total_weight <= 1e-12 {
        if enable_mode_heuristic || enable_minor_harmonic_bonus {
            detect_key_weighted_mode_heuristic(
                chroma_vectors,
                templates,
                frame_weights,
                mode_third_ratio_margin,
                if enable_mode_heuristic {
                    mode_flip_min_score_ratio
                } else {
                    0.0
                },
                enable_minor_harmonic_bonus,
                minor_leading_tone_bonus_weight,
            )
        } else {
            detect_key_weighted(chroma_vectors, templates, frame_weights)
        }
    } else {
        // Normalize accumulated scores by total weight
        for (_, score) in acc_scores.iter_mut() {
            *score /= total_weight;
        }
        
        // Sort and build result
        acc_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let (best_key, best_score) = acc_scores[0];
        let second_score = if acc_scores.len() > 1 { acc_scores[1].1 } else { 0.0 };
        let confidence = if best_score > 0.0 {
            ((best_score - second_score) / best_score).max(0.0).min(1.0)
        } else {
            0.0
        };
        
        let top_n = 3usize.min(acc_scores.len());
        let top_keys = acc_scores.iter().take(top_n).cloned().collect::<Vec<_>>();
        
        Ok(KeyDetectionResult {
            key: best_key,
            confidence,
            all_scores: acc_scores,
            top_keys,
        })
    }
}

/// Median key detection: detect key from multiple short segments and select median key.
///
/// This approach divides the track into multiple short overlapping segments, detects key
/// for each segment, and selects the median key (most common key across segments). This
/// helps handle brief modulations, breakdowns, or ambiguous sections that might bias
/// a global average.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `templates` - Key templates
/// * `frame_weights` - Optional per-frame weights
/// * `segment_length_frames` - Length of each segment in frames (default: ~4 seconds)
/// * `segment_hop_frames` - Hop size between segments in frames (default: ~1 second)
/// * `min_segments` - Minimum number of segments required (default: 3)
///
/// # Returns
///
/// Key detection result with median key across segments
pub fn detect_key_median(
    chroma_vectors: &[Vec<f32>],
    templates: &KeyTemplates,
    frame_weights: Option<&[f32]>,
    segment_length_frames: usize,
    segment_hop_frames: usize,
    min_segments: usize,
) -> Result<KeyDetectionResult, AnalysisError> {
    if chroma_vectors.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Empty chroma vectors".to_string()
        ));
    }
    
    let seg_len = segment_length_frames.min(chroma_vectors.len()).max(120);
    let hop = segment_hop_frames.max(1);
    let min_seg = min_segments.max(1);
    
    // Detect key for each segment
    let mut segment_keys = Vec::new();
    let mut segment_scores = Vec::new();
    let mut start = 0usize;
    
    while start + seg_len <= chroma_vectors.len() {
        let seg = &chroma_vectors[start..start + seg_len];
        let wseg = frame_weights.and_then(|w| {
            if start + seg_len <= w.len() {
                Some(&w[start..start + seg_len])
            } else {
                None
            }
        });
        
        match detect_key_weighted(seg, templates, wseg) {
            Ok(result) => {
                segment_keys.push(result.key);
                segment_scores.push(result);
            }
            Err(_) => {
                // Skip failed segments
            }
        }
        
        start += hop;
    }
    
    if segment_keys.len() < min_seg {
        // Fallback to global detection if not enough segments
        return detect_key_weighted(chroma_vectors, templates, frame_weights);
    }
    
    // Count key occurrences (weighted by confidence)
    let mut key_counts: std::collections::HashMap<Key, (usize, f32)> = std::collections::HashMap::new();
    for (key, result) in segment_keys.iter().zip(segment_scores.iter()) {
        let entry = key_counts.entry(*key).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += result.confidence;
    }
    
    // Select median key (most common, with confidence as tiebreaker)
    let (median_key, (count, _total_conf)) = key_counts.iter()
        .max_by(|a, b| {
            // First by count, then by total confidence
            match a.1.0.cmp(&b.1.0) {
                std::cmp::Ordering::Equal => a.1.1.partial_cmp(&b.1.1).unwrap_or(std::cmp::Ordering::Equal),
                other => other,
            }
        })
        .unwrap();
    
    // Compute aggregate scores from all segments for the median key
    let mut aggregate_scores: Vec<(Key, f32)> = Vec::with_capacity(24);
    for key_idx in 0..24 {
        let key = if key_idx < 12 {
            Key::Major(key_idx as u32)
        } else {
            Key::Minor((key_idx - 12) as u32)
        };
        
        // Sum scores for this key across all segments
        let mut total_score = 0.0f32;
        let mut total_weight = 0.0f32;
        for result in &segment_scores {
            if let Some((_, score)) = result.all_scores.iter().find(|(k, _)| *k == key) {
                total_score += *score * result.confidence;
                total_weight += result.confidence;
            }
        }
        
        let avg_score = if total_weight > 0.0 {
            total_score / total_weight
        } else {
            0.0
        };
        
        aggregate_scores.push((key, avg_score));
    }
    
    // Sort by aggregate score
    aggregate_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    // Use median key, but compute confidence from aggregate scores
    let median_score = aggregate_scores.iter()
        .find(|(k, _)| *k == *median_key)
        .map(|(_, s)| *s)
        .unwrap_or(0.0);
    let second_score = aggregate_scores.iter()
        .find(|(k, _)| *k != *median_key)
        .map(|(_, s)| *s)
        .unwrap_or(0.0);
    
    let confidence = if median_score > 0.0 {
        ((median_score - second_score) / median_score).max(0.0).min(1.0)
    } else {
        0.0
    };
    
    let top_n = 3;
    let top_keys: Vec<(Key, f32)> = aggregate_scores.iter()
        .take(top_n)
        .cloned()
        .collect();
    
    log::debug!("Median key detection: {:?} (appeared in {}/{} segments, confidence: {:.4})",
                median_key, count, segment_keys.len(), confidence);
    
    Ok(KeyDetectionResult {
        key: *median_key,
        confidence,
        all_scores: aggregate_scores,
        top_keys,
    })
}

/// Ensemble key detection: combine scores from multiple template sets.
///
/// This runs key detection with both Krumhansl-Kessler and Temperley templates,
/// then combines their scores using weighted voting. This ensemble approach can
/// improve robustness by leveraging complementary strengths of different template sets.
///
/// # Arguments
///
/// * `chroma_vectors` - Vector of 12-element chroma vectors (one per frame)
/// * `frame_weights` - Optional per-frame weights
/// * `kk_weight` - Weight for Krumhansl-Kessler template scores (default: 0.5)
/// * `temperley_weight` - Weight for Temperley template scores (default: 0.5)
///
/// # Returns
///
/// Key detection result with combined scores from both template sets.
pub fn detect_key_ensemble(
    chroma_vectors: &[Vec<f32>],
    frame_weights: Option<&[f32]>,
    kk_weight: f32,
    temperley_weight: f32,
) -> Result<KeyDetectionResult, AnalysisError> {
    // Normalize weights
    let total_weight = kk_weight + temperley_weight;
    let kk_norm = if total_weight > 1e-9 { kk_weight / total_weight } else { 0.5 };
    let temp_norm = if total_weight > 1e-9 { temperley_weight / total_weight } else { 0.5 };

    // Run detection with K-K templates
    let kk_templates = KeyTemplates::new_with_template_set(TemplateSet::KrumhanslKessler);
    let kk_result = detect_key_weighted(chroma_vectors, &kk_templates, frame_weights)?;

    // Run detection with Temperley templates
    let temp_templates = KeyTemplates::new_with_template_set(TemplateSet::Temperley);
    let temp_result = detect_key_weighted(chroma_vectors, &temp_templates, frame_weights)?;

    // Combine scores: weighted average of both template sets
    // Build a combined score map
    let mut combined_scores: Vec<(Key, f32)> = Vec::with_capacity(24);
    
    // Create score lookup maps
    let mut kk_scores = std::collections::HashMap::new();
    for (key, score) in &kk_result.all_scores {
        kk_scores.insert(*key, *score);
    }
    
    let mut temp_scores = std::collections::HashMap::new();
    for (key, score) in &temp_result.all_scores {
        temp_scores.insert(*key, *score);
    }

    // Combine scores for all 24 keys
    for key_idx in 0..24 {
        let key = if key_idx < 12 {
            Key::Major(key_idx as u32)
        } else {
            Key::Minor((key_idx - 12) as u32)
        };
        
        let kk_score = kk_scores.get(&key).copied().unwrap_or(0.0);
        let temp_score = temp_scores.get(&key).copied().unwrap_or(0.0);
        
        // Weighted combination
        let combined_score = kk_norm * kk_score + temp_norm * temp_score;
        combined_scores.push((key, combined_score));
    }

    // Sort by combined score (highest first)
    combined_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    // Select best key and compute confidence
    let (best_key, best_score) = combined_scores[0];
    let second_score = if combined_scores.len() > 1 { combined_scores[1].1 } else { 0.0 };
    
    // Confidence: (best - second) / best
    let confidence = if best_score > 0.0 {
        ((best_score - second_score) / best_score).max(0.0).min(1.0)
    } else {
        0.0
    };
    
    // Extract top N keys
    let top_n = 3;
    let top_keys: Vec<(Key, f32)> = combined_scores.iter()
        .take(top_n)
        .cloned()
        .collect();
    
    log::debug!("Ensemble detected key: {:?}, score: {:.4}, confidence: {:.4} (KK: {:.4}, Temp: {:.4})",
                best_key, best_score, confidence,
                kk_result.confidence, temp_result.confidence);
    
    Ok(KeyDetectionResult {
        key: best_key,
        confidence,
        all_scores: combined_scores,
        top_keys,
    })
}

/// Compute dot product between two vectors.
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x * y)
        .sum()
}

/// Sum dot products across frames, optionally applying per-frame weights.
fn weighted_sum_dot(
    chroma_vectors: &[Vec<f32>],
    weights: Option<&[f32]>,
    template: &[f32],
) -> f32 {
    let mut acc = 0.0f32;
    match weights {
        None => {
            for chroma in chroma_vectors {
                acc += dot_product(chroma, template);
            }
        }
        Some(w) => {
            for (chroma, &wt) in chroma_vectors.iter().zip(w.iter()) {
                if wt > 0.0 {
                    acc += wt * dot_product(chroma, template);
                }
            }
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_detect_key_empty() {
        let templates = KeyTemplates::new();
        let result = detect_key(&[], &templates);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_detect_key_basic() {
        let templates = KeyTemplates::new();
        
        // Create chroma vectors that match C major
        // C major template has high values at indices 0 (C), 4 (E), 7 (G)
        let mut chroma_vectors = Vec::new();
        for _ in 0..10 {
            let mut chroma = vec![0.0f32; 12];
            chroma[0] = 0.3; // C
            chroma[4] = 0.3; // E
            chroma[7] = 0.3; // G
            // Normalize
            let norm: f32 = chroma.iter().map(|&x| x * x).sum::<f32>().sqrt();
            for x in &mut chroma {
                *x /= norm;
            }
            chroma_vectors.push(chroma);
        }
        
        let result = detect_key(&chroma_vectors, &templates);
        assert!(result.is_ok());
        
        let detection = result.unwrap();
        assert!(detection.confidence >= 0.0 && detection.confidence <= 1.0);
        assert_eq!(detection.all_scores.len(), 24);
        
        // Best key should be C major (index 0)
        assert_eq!(detection.key, Key::Major(0));
        
        // Check top_keys is populated
        assert!(!detection.top_keys.is_empty());
        assert!(detection.top_keys.len() <= 3);
        assert_eq!(detection.top_keys[0].0, Key::Major(0));
    }
    
    #[test]
    fn test_detect_key_wrong_dimensions() {
        let templates = KeyTemplates::new();
        let chroma_vectors = vec![vec![0.0f32; 10]]; // Wrong size
        let result = detect_key(&chroma_vectors, &templates);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_average_chroma() {
        // Historical test placeholder: key detector no longer exposes average-chroma directly.
        // Keep a sanity check that weighted scoring behaves sensibly.
        let templates = KeyTemplates::new();
        let chroma_vectors = vec![vec![0.0f32; 12]; 10];
        let weights = vec![0.0f32; 10];
        let result = detect_key_weighted(&chroma_vectors, &templates, Some(&weights));
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let result = dot_product(&a, &b);
        assert_eq!(result, 32.0); // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    }
}
