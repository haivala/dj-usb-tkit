//! BPM/key analysis engine abstraction.
//!
//! Routes analysis to either the built-in stratum-dsp engine (pure Rust, no
//! external runtime) or the legacy essentia.js engine (requires Node.js).

use std::ops::Range;

use crate::error::{BackendError, BackendResult};

/// Which BPM/key detection engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisEngine {
    /// Built-in pure-Rust engine (stratum-dsp). No external runtime required.
    Stratum,
    /// Legacy essentia.js via Node.js shell-out.
    Essentia,
}

impl AnalysisEngine {
    /// Parse from setting string. Unknown values fall back to `Stratum`.
    pub fn from_setting(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "essentia" => Self::Essentia,
            _ => Self::Stratum,
        }
    }

    /// What a detected BPM's `bpm_analyzer` and a detected key's
    /// `tonality_source` are set to: the engine, plus the version of its
    /// steps when they have changed, so tracks analyzed before a change can be
    /// found. A bare `"stratum"` is stratum's raw estimate, before
    /// `refine_bpm`.
    pub fn bpm_analyzer(&self) -> &'static str {
        match self {
            Self::Stratum => "stratum-v3",
            Self::Essentia => "essentia",
        }
    }
}

/// Which engine detects the BPM (and places the first beat with it) and
/// which the key; each is chosen in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisEngines {
    pub bpm: AnalysisEngine,
    pub key: AnalysisEngine,
}

impl AnalysisEngines {
    #[cfg(test)]
    pub const STRATUM: Self = Self {
        bpm: AnalysisEngine::Stratum,
        key: AnalysisEngine::Stratum,
    };

    pub fn uses(&self, engine: AnalysisEngine) -> bool {
        self.bpm == engine || self.key == engine
    }
}

/// BPM/key detection result (engine-agnostic).
#[derive(Default)]
pub struct BpmKeyResult {
    pub bpm: Option<f64>,
    pub key: Option<String>,
    /// First beat (downbeat) position in milliseconds, from beat-grid analysis.
    pub first_beat_ms: Option<u32>,
}

fn empty_bpm_key_result() -> BpmKeyResult {
    BpmKeyResult {
        bpm: None,
        key: None,
        first_beat_ms: None,
    }
}

/// What to detect.
#[derive(Debug, Clone, Copy)]
pub struct DetectRequest {
    pub bpm_min: u32,
    pub bpm_max: u32,
    pub detect_bpm: bool,
    pub detect_key: bool,
}

/// What detection gets from a track's decode: a harmonic window and, when
/// available, a beat-rich early window. Each is short however long the track.
///
/// - `window`: the `KEY_WINDOW_SECS` whose waveform bins hold the most
///   mid-band energy (`key_window`), where the harmony sits. The key and one
///   BPM estimate come from it.
/// - `bpm_window`: the beat-rich early `BEAT_PIECE_SECS`, selected by rises
///   in raw low-band energy while decoding. It supplies a second BPM estimate.
/// - `envelope`: the whole track's onset envelope (`onset_envelope`), one
///   value per `ONSET_HOP` samples, a few hundred kilobytes for an hour.
///   `refine_bpm` pins the estimate on its two `beat_pieces`, and the first
///   beat is placed on its start (`place_first_beat`).
pub struct DetectionAudio {
    pub sample_rate: u32,
    pub window: Vec<f32>,
    /// Where `window` starts in the track, in samples.
    pub window_start: usize,
    pub bpm_window: Option<Vec<f32>>,
    pub envelope: Vec<f32>,
    pub beat_pieces: [Range<usize>; 2],
}

impl DetectionAudio {
    /// From all of a track's `samples` and its waveform bands, which spread
    /// evenly over them.
    #[cfg(test)]
    pub fn from_samples(
        samples: &[f32],
        sample_rate: u32,
        mid_energy: &[u8],
        low_energy: &[u8],
    ) -> Self {
        let window = key_window(
            mid_energy,
            samples.len(),
            KEY_WINDOW_SECS * sample_rate as usize,
        );
        Self {
            sample_rate,
            window_start: window.start,
            window: samples[window].to_vec(),
            bpm_window: None,
            envelope: onset_envelope(samples),
            beat_pieces: beat_pieces(low_energy, samples.len(), sample_rate),
        }
    }

    fn onsets(&self, ranges: &[Range<usize>]) -> Onsets {
        Onsets::from_envelope(&self.envelope, self.sample_rate, ranges)
    }

    /// The track's first `secs` seconds.
    fn start(&self, secs: usize) -> Onsets {
        let start = 0..secs * self.sample_rate as usize;
        self.onsets(std::slice::from_ref(&start))
    }

    /// Refine an estimate and return the beat-grid coherence that supports it.
    fn refine_bpm_with_coherence(&self, raw: f32, bpm_min: u32, bpm_max: u32) -> (f64, f64) {
        let onsets = self.onsets(&self.beat_pieces);
        let bpm = refine_bpm_onsets(
            &onsets,
            &self.start(BROAD_WINDOW_SECS),
            raw,
            bpm_min,
            bpm_max,
        );
        (bpm, subdivided_coherence(&onsets, bpm))
    }

    /// The first beat at `bpm` (`first_beat_secs`), placed on the track's
    /// first `FIRST_BEAT_WINDOW_SECS`; on its early beat piece when the start
    /// has no clear beat at that tempo (an intro of noise or pads). `None`
    /// for silence.
    pub fn place_first_beat(&self, bpm: f64) -> Option<f64> {
        let start = self.start(FIRST_BEAT_WINDOW_SECS);
        if has_clear_beat(&start, bpm) {
            return first_beat_secs_onsets(&start, bpm);
        }
        let early = self.onsets(&self.beat_pieces[..1]);
        first_beat_secs_onsets(&early, bpm).or_else(|| first_beat_secs_onsets(&start, bpm))
    }
}

/// Length of each `beat_pieces` stretch.
pub(super) const BEAT_PIECE_SECS: usize = 120;
/// How far apart the beat pieces' starts may lie: the beat count between
/// them must stay unambiguous for stratum's estimate, and a long mix's tempo
/// may wander.
const BEAT_SPAN_MAX_SECS: usize = 15 * 60;

/// The two stretches `refine_bpm` pins the tempo on, in samples of a track
/// `total` long: the most beat-like `BEAT_PIECE_SECS` in its first half, and
/// the most beat-like after that, starting at most `BEAT_SPAN_MAX_SECS` from
/// the first. A tempo 0.01 BPM off moves a grid only 0.03 beats in three
/// minutes, but the further apart the two lie, the further it moves between
/// them, so they tell 0.01 BPM apart as well as all the audio between them.
/// `low_energy` is the waveform's low band, its bins spread evenly over the
/// track (`beat_strength`).
pub fn beat_pieces(low_energy: &[u8], total: usize, sample_rate: u32) -> [Range<usize>; 2] {
    let piece = BEAT_PIECE_SECS * sample_rate as usize;
    let strength = beat_strength(low_energy);
    let early = best_window(&strength, total, 0..total.div_ceil(2), piece);
    let late_end = total.min(early.start + BEAT_SPAN_MAX_SECS * sample_rate as usize);
    let late = best_window(&strength, total, early.end.min(late_end)..late_end, piece);
    [early, late]
}

/// How beat-like each waveform bin is: the rise of its low-band energy over
/// the bin before. A kick rises sharply every beat; a held bassline or pad
/// is loud but barely rises.
fn beat_strength(low_energy: &[u8]) -> Vec<u8> {
    std::iter::once(0)
        .chain(low_energy.windows(2).map(|p| p[1].saturating_sub(p[0])))
        .collect()
}

/// The `window` samples within `within` (of `total`, which `score`'s bins
/// spread evenly over) whose bins score highest, as `key_window` picks.
fn best_window(score: &[u8], total: usize, within: Range<usize>, window: usize) -> Range<usize> {
    let bins = score.len();
    if bins == 0 || total == 0 {
        return within.start..(within.start + window).min(within.end);
    }
    let first = (within.start as u128 * bins as u128 / total as u128) as usize;
    let last = ((within.end as u128 * bins as u128).div_ceil(total as u128) as usize).min(bins);
    let picked = key_window(&score[first..last.max(first)], within.len(), window);
    within.start + picked.start..(within.start + picked.end).min(within.end)
}

/// Run BPM/key detection using the stratum-dsp engine; what isn't asked for
/// is left out (`None`) and costs nothing. BPM compares estimates from the
/// harmonic and beat-rich windows, pinning either on the same beat pieces.
pub fn detect_bpm_key_stratum(
    audio: &DetectionAudio,
    request: &DetectRequest,
) -> BackendResult<BpmKeyResult> {
    let sample_rate = audio.sample_rate;
    if audio.window.is_empty() || sample_rate == 0 {
        return Ok(empty_bpm_key_result());
    }
    let failed = |e: stratum_dsp::AnalysisError| {
        BackendError::Internal(format!("stratum-dsp analysis failed: {e}"))
    };
    let (bpm_min, bpm_max) = (request.bpm_min, request.bpm_max);

    let (bpm, first_beat_ms) = if request.detect_bpm {
        let result =
            stratum_dsp::analyze_bpm(&audio.window, sample_rate, stratum_config(bpm_min, bpm_max))
                .map_err(failed)?;
        let harmonic = (result.bpm > 0.0)
            .then(|| audio.refine_bpm_with_coherence(result.bpm, bpm_min, bpm_max));
        let beat = audio
            .bpm_window
            .as_deref()
            .filter(|window| !window.is_empty())
            .map(|window| {
                stratum_dsp::analyze_bpm(window, sample_rate, stratum_config(bpm_min, bpm_max))
                    .map_err(failed)
                    .map(|result| (result.bpm, result.bpm > 0.0))
            })
            .transpose()?
            .and_then(|(raw, valid)| valid.then(|| audio.refine_bpm_with_coherence(raw, bpm_min, bpm_max)));
        // A second estimate may disagree because it saw drums rather than
        // harmony. Let it replace the harmonic estimate only with a clear
        // beat-grid advantage; equal evidence keeps the established path.
        let bpm = match (harmonic, beat) {
            (Some((_harmonic, harmonic_score)), Some((beat, beat_score)))
                if beat_score >= harmonic_score * 1.2 => Some(beat),
            (Some((harmonic, _)), _) => Some(harmonic),
            (None, Some((beat, _))) => Some(beat),
            (None, None) => None,
        };
        // stratum places its beats at its own estimate, which `refine_bpm` may
        // have moved by up to 2 BPM or to a 3:2 / 4:3 relative; its first beat
        // is also often well off a beat.
        let window_start_secs = audio.window_start as f64 / f64::from(sample_rate);
        let first_beat_ms = bpm
            .and_then(|bpm| audio.place_first_beat(bpm))
            .or_else(|| {
                result
                    .beat_grid
                    .beats
                    .first()
                    .map(|&t| f64::from(t) + window_start_secs)
            })
            .map(|t| (t * 1000.0).round() as u32);
        (bpm, first_beat_ms)
    } else {
        (None, None)
    };

    let key = if request.detect_key {
        let result = stratum_dsp::analyze_key(
            &audio.window,
            sample_rate,
            stratum_key_config(bpm_min, bpm_max),
        )
        .map_err(failed)?;
        Some(result.key.name()).filter(|name| !name.is_empty())
    } else {
        None
    };

    Ok(BpmKeyResult {
        bpm,
        key,
        first_beat_ms,
    })
}

pub(super) fn stratum_config(bpm_min: u32, bpm_max: u32) -> stratum_dsp::AnalysisConfig {
    stratum_dsp::AnalysisConfig {
        min_bpm: bpm_min as f32,
        max_bpm: bpm_max as f32,
        ..Default::default()
    }
}

/// How much of the track the key is detected from.
pub(super) const KEY_WINDOW_SECS: usize = 180;

/// stratum's key settings, chosen by `key_sweep` (key_reference_eval.rs) on
/// the two GiantSteps key sets, picked on one and confirmed on the other:
/// Temperley's key profiles, the key voted over stretches of 60, 120 and
/// 360 spectrogram frames (multi-scale), the bass band blended into the pitch profile,
/// tuning compensation, and the profile built from the 12 strongest spectral
/// peaks per frame, by magnitude, weighting every frame alike. Each helped
/// on both sets, and together they find the exact key on over half again as
/// many tracks as stratum's defaults, at the same speed. Without the
/// spectrogram time smoothing and the harmonic mask: they take about two
/// thirds of the key time and added nothing on these sets.
pub(super) fn stratum_key_config(bpm_min: u32, bpm_max: u32) -> stratum_dsp::AnalysisConfig {
    stratum_dsp::AnalysisConfig {
        enable_key_spectrogram_time_smoothing: false,
        enable_key_harmonic_mask: false,
        key_template_set: stratum_dsp::features::key::templates::TemplateSet::Temperley,
        enable_key_multi_scale: true,
        key_multi_scale_lengths: vec![60, 120, 360],
        enable_key_hpcp_bass_blend: true,
        enable_key_tuning_compensation: true,
        key_hpcp_mag_power: 1.0,
        key_hpcp_peaks_per_frame: 12,
        key_energy_power: 0.0,
        ..stratum_config(bpm_min, bpm_max)
    }
}

/// The `window` samples (of `total`) whose waveform bins hold the most
/// mid-band energy, where a track's harmony sits; the earliest such window
/// on a tie, all of it when it is no longer than `window`. The bins spread
/// evenly over the `total` samples.
pub(super) fn key_window(mid_energy: &[u8], total: usize, window: usize) -> Range<usize> {
    let bins = mid_energy.len();
    if total <= window || bins == 0 {
        return 0..total.min(window);
    }
    let width = (window * bins).div_ceil(total).clamp(1, bins);
    let mut sum: u64 = mid_energy[..width].iter().map(|&v| u64::from(v)).sum();
    let (mut best, mut best_start) = (sum, 0);
    for start in 1..=bins - width {
        sum = sum + u64::from(mid_energy[start + width - 1]) - u64::from(mid_energy[start - 1]);
        if sum > best {
            (best, best_start) = (sum, start);
        }
    }
    let start = (best_start * total / bins).min(total - window);
    start..start + window
}

/// Hop between onset-envelope frames, in samples.
const ONSET_HOP: usize = 512;
/// How far around stratum's estimate to look: its typical error is ±2 BPM.
const BPM_SEARCH_RADIUS: f64 = 2.0;
/// Resolution of the search; rekordbox stores BPMs to 0.01.
const BPM_SEARCH_STEP: f64 = 0.01;
/// The best tempo's beat coherence must reach this, and beat every tempo at
/// least `RIVAL_DISTANCE` away by `MIN_COHERENCE_LEAD`, to be trusted over
/// plain rounding.
const MIN_BEAT_COHERENCE: f64 = 0.06;
const MIN_COHERENCE_LEAD: f64 = 1.5;
const RIVAL_DISTANCE: f64 = 0.5;
/// A tempo is scored on the beat and on its 2× and 4× subdivisions. At the
/// beat alone, a backbeat half a beat off cancels part of the score and
/// blunts the peak (a drum & bass snare when stratum answers 87 for 174);
/// on the subdivisions it lands in phase, and the peak gets sharper.
const SUBDIVISIONS: [f64; 3] = [1.0, 2.0, 4.0];
/// The nearest whole or half BPM is kept when it is this coherent relative
/// to the best tempo: on a track whose tempo is a whole or half BPM, the
/// best tempo can still land a few hundredths off it.
const GRID_SNAP_COHERENCE: f64 = 0.9;
/// stratum sometimes answers a tempo related to the real one by 3:2, 4:3 or
/// 5:4 (83 for 125, 116 for 174, 156 for 125 on a short stretch of audio).
/// At such a tempo the beats land on two to four different phases and
/// cancel out, so the real one scores clearly higher. Halving and doubling
/// are left alone: there, every onset still lands on the grid and the score
/// can't tell which is right.
///
/// Each factor comes with how many times more coherent the related tempo
/// must be than stratum's to replace it. 3/2 and 3/4 need far more: one of
/// their `SUBDIVISIONS` is 3× stratum's tempo, where every one of its beats
/// lands in phase too, so they score well even when they are wrong.
///
/// Stepping down by 4:5 (stratum hearing 160 for 128, 175 for 140) needs
/// less than stepping up: on a public set of crowd-annotated tempos, the
/// real tempo under such a mistake scored 0.68 to 2.37 times as coherent,
/// and down to 1.5 no track that was right turned wrong. Stepping up by 5:4
/// at that lead would move tracks at 87-91 BPM to 108-114.
pub(super) const TEMPO_FAMILY: [(f64, f64); 6] = [
    (1.5, 8.0),
    (0.75, 8.0),
    (2.0 / 3.0, 2.5),
    (4.0 / 3.0, 2.5),
    (0.8, 1.6),
    (1.25, 2.5),
];

/// Turn stratum's raw estimate into the BPM stored for the track.
///
/// stratum searches whole-BPM steps and is typically off by up to ±2, so its
/// decimals mean nothing on their own. Instead, every tempo within ±2 of it
/// is tried, in 0.01 steps, against the whole track: a tempo even 0.1 BPM off
/// drifts a beat over ten minutes, so only the right one keeps the onsets in
/// phase. That finds tempos between whole and half values (174.79) as well
/// as whole and half ones. When no tempo clearly wins, the whole and half
/// BPMs alone are compared the same way; when none of those wins either
/// (ambiguous rhythm, tempo changes), the estimate is rounded to a whole BPM.
/// stratum's octave is kept as is, but a tempo related to it by 3:2, 4:3 or
/// 5:4 replaces it when that one clearly fits the track better
/// (`TEMPO_FAMILY`).
#[cfg(test)]
pub(super) fn refine_bpm(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> f64 {
    refine_bpm_onsets(
        &Onsets::of(samples, sample_rate),
        &Onsets::of(broad_window(samples, sample_rate), sample_rate),
        raw,
        bpm_min,
        bpm_max,
    )
}

/// `refine_bpm` over `onsets`; `start` is the onset envelope of the track's
/// first `BROAD_WINDOW_SECS`, for the broad search.
pub(super) fn refine_bpm_onsets(
    onsets: &Onsets,
    start: &Onsets,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> f64 {
    let local_peak = find_tempo_peak_onsets(onsets, raw, bpm_min, bpm_max);
    if local_peak.is_none()
        && let Some(candidate) = broad_tempo_candidate_onsets(start, raw, bpm_min, bpm_max)
    {
        return refine_near_estimate_onsets(onsets, candidate, bpm_min, bpm_max);
    }
    let estimate = match tempo_family_alternative_onsets(onsets, raw, bpm_min, bpm_max) {
        Some((alternative, margin)) if margin >= 1.0 => alternative,
        _ => raw,
    };
    if estimate == raw {
        bpm_from_peak(local_peak, raw)
    } else {
        refine_near_estimate_onsets(onsets, estimate, bpm_min, bpm_max)
    }
}

/// The onset envelope (`onset_envelope`) of some stretches of a track, each
/// with the frame it starts at, so a beat grid laid at one tempo lines up
/// across the gaps between them. The tempo is only as precise as the stretches
/// are far apart: an error of 0.01 BPM moves the grid 0.03 beats in three
/// minutes, 0.1 in ten.
pub(super) struct Onsets {
    frame_secs: f64,
    /// First frame (`ONSET_HOP` samples each) and the envelope from there.
    parts: Vec<(usize, Vec<f32>)>,
}

impl Onsets {
    /// All of `samples`, from the track's start.
    #[cfg(test)]
    pub(super) fn of(samples: &[f32], sample_rate: u32) -> Self {
        Self {
            frame_secs: ONSET_HOP as f64 / f64::from(sample_rate),
            parts: vec![(0, onset_envelope(samples))],
        }
    }

    /// The `ranges` (sample positions in the track) of a whole track's onset
    /// `envelope`, from their first whole frame; overlapping ones are merged.
    /// Each starts at 0, as `onset_envelope` of that stretch alone would.
    pub(super) fn from_envelope(
        envelope: &[f32],
        sample_rate: u32,
        ranges: &[Range<usize>],
    ) -> Self {
        let mut frames = ranges
            .iter()
            .map(|r| r.start / ONSET_HOP..r.end.div_ceil(ONSET_HOP).min(envelope.len()))
            .filter(|r| r.start < r.end)
            .collect::<Vec<_>>();
        frames.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in frames {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Self {
            frame_secs: ONSET_HOP as f64 / f64::from(sample_rate),
            parts: merged
                .into_iter()
                .map(|r| {
                    let mut part = envelope[r.clone()].to_vec();
                    part[0] = 0.0;
                    (r.start, part)
                })
                .collect(),
        }
    }
}

/// How much of the track's start the broad search looks at.
pub(super) const BROAD_WINDOW_SECS: usize = 120;

/// The first `BROAD_WINDOW_SECS` of `samples`.
#[cfg(test)]
pub(super) fn broad_window(samples: &[f32], sample_rate: u32) -> &[f32] {
    &samples[..samples.len().min(sample_rate as usize * BROAD_WINDOW_SECS)]
}

/// `refine_bpm` without the tempo family check.
#[cfg(test)]
pub(super) fn refine_near_estimate(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> f64 {
    refine_near_estimate_onsets(&Onsets::of(samples, sample_rate), raw, bpm_min, bpm_max)
}

fn refine_near_estimate_onsets(onsets: &Onsets, raw: f32, bpm_min: u32, bpm_max: u32) -> f64 {
    bpm_from_peak(find_tempo_peak_onsets(onsets, raw, bpm_min, bpm_max), raw)
}

fn bpm_from_peak(peak: Option<TempoPeak>, raw: f32) -> f64 {
    match peak {
        None => f64::from(raw.round()),
        Some(peak) if peak.grid_coherence >= GRID_SNAP_COHERENCE * peak.coherence => peak.grid,
        Some(peak) => (peak.bpm * 100.0).round() / 100.0,
    }
}

/// Rescue a strong off-grid tempo when the full-track search around stratum's
/// estimate has no clear peak. A short window lets us search more broadly at
/// 0.05 BPM resolution without slowing every successful local search. The
/// full track is still used by `refine_near_estimate` for the final BPM.
#[cfg(test)]
pub(super) fn broad_tempo_candidate(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<f32> {
    broad_tempo_candidate_onsets(
        &Onsets::of(broad_window(samples, sample_rate), sample_rate),
        raw,
        bpm_min,
        bpm_max,
    )
}

/// `broad_tempo_candidate` over `start`, the onsets of the track's first
/// `BROAD_WINDOW_SECS`.
fn broad_tempo_candidate_onsets(
    start: &Onsets,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<f32> {
    const STEP: f64 = 0.05;
    const MIN_SCORE: f64 = 0.1;
    const MIN_LEAD: f64 = 1.25;

    let (min, max) = (f64::from(bpm_min), f64::from(bpm_max));
    let raw = f64::from(raw);
    let candidates = [
        (1.0, 10.0),
        (1.5, 3.0),
        (0.75, 2.0),
        (2.0 / 3.0, 2.0),
        (4.0 / 3.0, 3.0),
        (0.8, 2.0),
        (1.25, 3.0),
    ]
    .iter()
    .filter_map(|&(factor, radius)| {
        let centre = raw * factor;
        let steps = (radius / STEP).round() as i32;
        (-steps..=steps)
            .map(|step| centre + f64::from(step) * STEP)
            .filter(|&bpm| bpm >= min && bpm <= max)
            .map(|bpm| (bpm, subdivided_coherence(start, bpm)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
    })
    .collect::<Vec<_>>();
    let &(bpm, score) = candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    let rival = candidates
        .iter()
        .filter(|&&(other, _)| {
            (other - bpm).abs() >= RIVAL_DISTANCE
                && (other * 2.0 - bpm).abs() >= RIVAL_DISTANCE
                && (other / 2.0 - bpm).abs() >= RIVAL_DISTANCE
        })
        .map(|&(_, score)| score)
        .fold(0.0, f64::max);
    (score >= MIN_SCORE && score >= MIN_LEAD * rival).then_some(bpm as f32)
}

/// The tempo related to stratum's estimate by 3:2, 4:3 or 5:4 (`TEMPO_FAMILY`)
/// that comes closest to replacing it, and how close: its coherence lead
/// over the estimate divided by the lead its factor requires, so 1.0 or
/// more means it should replace the estimate. Both are compared at their
/// best whole or half BPM within the search radius, which is enough to
/// tell tempo families apart. The winning BPM is passed to the fine search
/// so it stays within that search's radius.
#[cfg(test)]
pub(super) fn tempo_family_alternative(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<(f32, f64)> {
    tempo_family_alternative_onsets(&Onsets::of(samples, sample_rate), raw, bpm_min, bpm_max)
}

fn tempo_family_alternative_onsets(
    onsets: &Onsets,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<(f32, f64)> {
    tempo_family_leads_onsets(onsets, raw, bpm_min, bpm_max)?
        .into_iter()
        .zip(TEMPO_FAMILY)
        .filter_map(|(l, (_, required))| l.map(|l| (l.bpm, l.lead / required)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// One `TEMPO_FAMILY` relative of stratum's estimate.
pub(super) struct FamilyLead {
    /// Its best whole or half BPM within the search radius.
    pub bpm: f32,
    /// How many times as coherent as the estimate it is.
    pub lead: f64,
}

/// Each `TEMPO_FAMILY` relative of stratum's estimate, in `TEMPO_FAMILY`'s
/// order, compared with the estimate at their best whole or half BPM; a
/// relative outside the analysis range is `None`. `None` altogether when
/// the estimate has no coherence at all.
#[cfg(test)]
pub(super) fn tempo_family_leads(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<Vec<Option<FamilyLead>>> {
    tempo_family_leads_onsets(&Onsets::of(samples, sample_rate), raw, bpm_min, bpm_max)
}

fn tempo_family_leads_onsets(
    onsets: &Onsets,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<Vec<Option<FamilyLead>>> {
    let (min, max) = (f64::from(bpm_min), f64::from(bpm_max));
    let raw = f64::from(raw);
    let best_near = |centre: f64, radius: f64| {
        let steps = (radius * 2.0).round() as i32;
        (-steps..=steps)
            .map(|step| (centre * 2.0).round() / 2.0 + f64::from(step) * 0.5)
            .filter(|&bpm| bpm >= min && bpm <= max)
            .map(|bpm| (bpm, subdivided_coherence(onsets, bpm)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
    };
    let own = best_near(raw, BPM_SEARCH_RADIUS)?.1;
    if own <= 0.0 {
        return None;
    }
    Some(
        TEMPO_FAMILY
            .iter()
            .map(|&(factor, _)| {
                // stratum's error scales with the factor.
                let (bpm, score) = best_near(raw * factor, BPM_SEARCH_RADIUS * factor.max(1.0))?;
                Some(FamilyLead {
                    bpm: bpm as f32,
                    lead: score / own,
                })
            })
            .collect(),
    )
}

/// The tempo that clearly keeps the onsets in phase best, near stratum's
/// estimate, and the nearest whole or half BPM to it.
pub(super) struct TempoPeak {
    pub bpm: f64,
    pub coherence: f64,
    pub grid: f64,
    pub grid_coherence: f64,
}

/// `None` when no tempo clearly wins, not even among whole and half BPMs.
#[cfg(test)]
pub(super) fn find_tempo_peak(
    samples: &[f32],
    sample_rate: u32,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<TempoPeak> {
    find_tempo_peak_onsets(&Onsets::of(samples, sample_rate), raw, bpm_min, bpm_max)
}

fn find_tempo_peak_onsets(
    onsets: &Onsets,
    raw: f32,
    bpm_min: u32,
    bpm_max: u32,
) -> Option<TempoPeak> {
    let coherence = |bpm: f64| subdivided_coherence(onsets, bpm);
    let steps = (BPM_SEARCH_RADIUS / BPM_SEARCH_STEP).round() as i32;
    let scored = (-steps..=steps)
        .map(|step| f64::from(raw.round()) + f64::from(step) * BPM_SEARCH_STEP)
        .filter(|&bpm| bpm >= f64::from(bpm_min) && bpm <= f64::from(bpm_max))
        .map(|bpm| (bpm, coherence(bpm)))
        .collect::<Vec<_>>();
    let clear_winner = |candidates: &[(f64, f64)]| {
        let &(bpm, score) = candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
        let rival = candidates
            .iter()
            .filter(|(other, _)| (other - bpm).abs() >= RIVAL_DISTANCE)
            .map(|&(_, score)| score)
            .fold(0.0, f64::max);
        (score >= MIN_BEAT_COHERENCE && score >= MIN_COHERENCE_LEAD * rival).then_some((bpm, score))
    };
    let Some((bpm, score)) = clear_winner(&scored) else {
        // A rhythm too loose for a sharp peak can still clearly favour one
        // whole or half BPM over its neighbours.
        let grid_only = scored
            .iter()
            .filter(|(bpm, _)| (bpm * 2.0 - (bpm * 2.0).round()).abs() < 1e-6)
            .map(|&(bpm, score)| ((bpm * 2.0).round() / 2.0, score))
            .collect::<Vec<_>>();
        return clear_winner(&grid_only).map(|(grid, score)| TempoPeak {
            bpm: grid,
            coherence: score,
            grid,
            grid_coherence: score,
        });
    };
    let grid = (bpm * 2.0).round() / 2.0;
    Some(TempoPeak {
        bpm,
        coherence: score,
        grid,
        grid_coherence: coherence(grid),
    })
}

/// How well `onsets` keep in phase at `bpm` (`subdivided_coherence`); a
/// beat is clear at `MIN_BEAT_COHERENCE` or more.
pub(super) fn has_clear_beat(onsets: &Onsets, bpm: f64) -> bool {
    subdivided_coherence(onsets, bpm) >= MIN_BEAT_COHERENCE
}

/// `beat_coherence` averaged over the beat and its `SUBDIVISIONS`.
fn subdivided_coherence(onsets: &Onsets, bpm: f64) -> f64 {
    SUBDIVISIONS
        .iter()
        .map(|m| beat_coherence(onsets, bpm * m))
        .sum::<f64>()
        / SUBDIVISIONS.len() as f64
}

/// Rise in signal energy per `ONSET_HOP` frame (negative changes dropped):
/// peaks where notes and drums start.
pub(super) fn onset_envelope(samples: &[f32]) -> Vec<f32> {
    let energy = samples
        .chunks(ONSET_HOP)
        .map(|frame| frame.iter().map(|x| x * x).sum::<f32>())
        .collect::<Vec<_>>();
    envelope_of_energies(&energy)
}

fn envelope_of_energies(energy: &[f32]) -> Vec<f32> {
    if energy.is_empty() {
        return Vec::new();
    }
    std::iter::once(0.0)
        .chain(energy.windows(2).map(|pair| (pair[1] - pair[0]).max(0.0)))
        .collect()
}

/// `onset_envelope` built as a track decodes, chunk by chunk, keeping
/// nothing but one energy per `ONSET_HOP` samples.
#[derive(Default)]
pub struct OnsetAccumulator {
    energies: Vec<f32>,
    energy: f32,
    filled: usize,
}

impl OnsetAccumulator {
    pub fn push(&mut self, samples: &[f32]) {
        for &x in samples {
            self.energy += x * x;
            self.filled += 1;
            if self.filled == ONSET_HOP {
                self.energies.push(self.energy);
                (self.energy, self.filled) = (0.0, 0);
            }
        }
    }

    /// `onset_envelope` of everything pushed.
    pub fn finish(mut self) -> Vec<f32> {
        if self.filled > 0 {
            self.energies.push(self.energy);
        }
        envelope_of_energies(&self.energies)
    }
}

/// The part of the track the first beat is placed from: the grid has to fit
/// where it starts, and a tempo that wanders a little moves the phase
/// measured over the whole track away from the start.
pub(super) const FIRST_BEAT_WINDOW_SECS: usize = 60;

/// Time of the first beat at `bpm`, within the first beat period: the phase
/// the onsets share on the beat picks which beat it is, and the phase on the
/// 2× and then 4× subdivisions, whose onsets are sharper and more frequent,
/// then places it more precisely. `None` for a silent start.
#[cfg(test)]
pub(super) fn first_beat_secs(samples: &[f32], sample_rate: u32, bpm: f64) -> Option<f64> {
    let window = &samples[..samples
        .len()
        .min(sample_rate as usize * FIRST_BEAT_WINDOW_SECS)];
    first_beat_secs_onsets(&Onsets::of(window, sample_rate), bpm)
}

/// `first_beat_secs` over `start`, the onsets of the track's first
/// `FIRST_BEAT_WINDOW_SECS`.
pub(super) fn first_beat_secs_onsets(start: &Onsets, bpm: f64) -> Option<f64> {
    let period = 60.0 / bpm;
    let mut first = None::<f64>;
    for multiple in [1.0, 2.0, 4.0] {
        let (re, im, total) = beat_phasor(start, bpm * multiple);
        if total <= 0.0 {
            return None;
        }
        let step = period / multiple;
        let offset = im.atan2(re).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * step;
        // The subdivision onset nearest the beat found so far.
        first = Some(first.map_or(offset, |t| offset + ((t - offset) / step).round() * step));
    }
    first.map(|t| t.rem_euclid(period))
}

/// How consistently the onsets fall on the same beat phase at `bpm`, over
/// all of `onsets`: 1.0 when every onset lands on the grid, near 0 when the
/// grid drifts through them.
fn beat_coherence(onsets: &Onsets, bpm: f64) -> f64 {
    let (re, im, total) = beat_phasor(onsets, bpm);
    if total <= 0.0 {
        return 0.0;
    }
    (re * re + im * im).sqrt() / total
}

/// The onset envelope summed as vectors whose angle is each frame's beat
/// phase at `bpm`, and the plain sum of its weights.
fn beat_phasor(onsets: &Onsets, bpm: f64) -> (f64, f64, f64) {
    let step = std::f64::consts::TAU * onsets.frame_secs * bpm / 60.0;
    // The beat phase advances by a fixed angle per frame: rotate a unit
    // vector instead of evaluating sin/cos for every frame.
    let (step_sin, step_cos) = step.sin_cos();
    let (mut re, mut im, mut total) = (0.0f64, 0.0f64, 0.0f64);
    for (first, envelope) in &onsets.parts {
        // Each stretch starts at its own phase; the first at frame 0 at 0.
        let (mut sin, mut cos) = if *first == 0 {
            (0.0, 1.0)
        } else {
            (step * *first as f64)
                .rem_euclid(std::f64::consts::TAU)
                .sin_cos()
        };
        for &weight in envelope {
            let weight = f64::from(weight);
            re += weight * cos;
            im += weight * sin;
            total += weight;
            (cos, sin) = (
                cos * step_cos - sin * step_sin,
                sin * step_cos + cos * step_sin,
            );
        }
    }
    (re, im, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_from_setting_defaults_to_stratum() {
        assert_eq!(AnalysisEngine::from_setting(""), AnalysisEngine::Stratum);
        assert_eq!(
            AnalysisEngine::from_setting("stratum"),
            AnalysisEngine::Stratum
        );
        assert_eq!(
            AnalysisEngine::from_setting("unknown"),
            AnalysisEngine::Stratum
        );
    }

    #[test]
    fn engine_from_setting_essentia() {
        assert_eq!(
            AnalysisEngine::from_setting("essentia"),
            AnalysisEngine::Essentia
        );
        assert_eq!(
            AnalysisEngine::from_setting("Essentia"),
            AnalysisEngine::Essentia
        );
        assert_eq!(
            AnalysisEngine::from_setting("  ESSENTIA  "),
            AnalysisEngine::Essentia
        );
    }

    const BOTH: DetectRequest = DetectRequest {
        bpm_min: 70,
        bpm_max: 180,
        detect_bpm: true,
        detect_key: true,
    };

    /// All of `samples`, with no waveform: the window is their start.
    fn audio(samples: &[f32], sample_rate: u32) -> DetectionAudio {
        DetectionAudio::from_samples(samples, sample_rate, &[], &[])
    }

    #[test]
    fn key_window_takes_the_most_mid_energy() {
        // 10 bins over 1000 samples; a 300-sample window spans 3 bins.
        let mid = [1, 1, 1, 1, 9, 9, 9, 1, 1, 1];
        assert_eq!(key_window(&mid, 1000, 300), 400..700);
        // A tie keeps the earliest; the window never runs past the end.
        assert_eq!(key_window(&[5; 10], 1000, 300), 0..300);
        assert_eq!(
            key_window(&[0, 0, 0, 0, 0, 0, 0, 0, 9, 9], 1000, 300),
            700..1000
        );
        // A short track, or no waveform: from the start.
        assert_eq!(key_window(&mid, 200, 300), 0..200);
        assert_eq!(key_window(&[], 1000, 300), 0..300);
    }

    #[test]
    fn stratum_detects_only_what_is_asked() {
        let samples = click_track_secs(124.0, 44100, 30);
        let bpm_only = detect_bpm_key_stratum(
            &audio(&samples, 44100),
            &DetectRequest {
                detect_key: false,
                ..BOTH
            },
        )
        .unwrap();
        assert!(bpm_only.bpm.is_some() && bpm_only.first_beat_ms.is_some());
        assert!(bpm_only.key.is_none());
        let key_only = detect_bpm_key_stratum(
            &audio(&samples, 44100),
            &DetectRequest {
                detect_bpm: false,
                ..BOTH
            },
        )
        .unwrap();
        assert!(key_only.bpm.is_none() && key_only.first_beat_ms.is_none());
        assert!(key_only.key.is_some());
    }

    #[test]
    fn stratum_empty_samples_returns_none() {
        let result = detect_bpm_key_stratum(&audio(&[], 44100), &BOTH).unwrap();
        assert!(result.bpm.is_none());
        assert!(result.key.is_none());
        assert!(result.first_beat_ms.is_none());
    }

    #[test]
    fn stratum_zero_sample_rate_returns_none() {
        let result = detect_bpm_key_stratum(&audio(&[0.1, 0.2, 0.3], 0), &BOTH).unwrap();
        assert!(result.bpm.is_none());
        assert!(result.key.is_none());
        assert!(result.first_beat_ms.is_none());
    }

    #[test]
    fn stratum_sine_produces_result() {
        // Generate a 10-second 440Hz sine wave at 44100Hz — should detect key of A
        let sample_rate = 44100u32;
        let duration_secs = 10.0f32;
        let freq = 440.0f32;
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin() * 0.8
            })
            .collect();

        let result = detect_bpm_key_stratum(&audio(&samples, sample_rate), &BOTH).unwrap();
        // A pure sine won't have clear BPM, but key detection should produce something
        assert!(result.key.is_some(), "expected key detection on sine wave");
    }

    /// The vendored stratum's BPM-only and key-only entry points give exactly
    /// what `analyze_audio` gives for its half.
    #[test]
    fn stratum_split_entry_points_match_full_analysis() {
        let sample_rate = 44100u32;
        let mut samples = click_track_secs(124.0, sample_rate, 30);
        // An A minor triad under the clicks, for the key.
        for (i, s) in samples.iter_mut().enumerate() {
            let t = i as f32 / sample_rate as f32;
            *s += [220.0f32, 261.63, 329.63]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin() * 0.15)
                .sum::<f32>();
        }
        let config = || stratum_config(70, 180);
        let full = stratum_dsp::analyze_audio(&samples, sample_rate, config()).unwrap();
        let bpm = stratum_dsp::analyze_bpm(&samples, sample_rate, config()).unwrap();
        let key = stratum_dsp::analyze_key(&samples, sample_rate, config()).unwrap();

        assert!(full.bpm > 0.0, "the full analysis should find a tempo");
        assert_eq!(bpm.bpm.to_bits(), full.bpm.to_bits());
        assert_eq!(bpm.bpm_confidence.to_bits(), full.bpm_confidence.to_bits());
        assert_eq!(bpm.beat_grid.beats, full.beat_grid.beats);
        assert_eq!(bpm.grid_stability.to_bits(), full.grid_stability.to_bits());

        assert_eq!(key.key, full.key);
        assert_eq!(key.key_confidence.to_bits(), full.key_confidence.to_bits());
        assert_eq!(key.key_clarity.to_bits(), full.key_clarity.to_bits());
        assert_eq!(key.bpm, 0.0);
        assert!(key.beat_grid.beats.is_empty());
    }

    /// Two minutes of kick-like clicks at `bpm`.
    fn click_track(bpm: f64, sample_rate: u32) -> Vec<f32> {
        click_track_secs(bpm, sample_rate, 120)
    }

    /// `secs` seconds of kick-like clicks at `bpm`.
    fn click_track_secs(bpm: f64, sample_rate: u32, secs: usize) -> Vec<f32> {
        let mut samples = vec![0f32; sample_rate as usize * secs];
        let interval = 60.0 / bpm * f64::from(sample_rate);
        let mut beat = 0.0;
        while (beat * interval) as usize + 2000 < samples.len() {
            let start = (beat * interval) as usize;
            for j in 0..2000 {
                let t = j as f32 / sample_rate as f32;
                samples[start + j] +=
                    (-(j as f32) / 300.0).exp() * (std::f32::consts::TAU * 60.0 * t).sin();
            }
            beat += 1.0;
        }
        samples
    }

    #[test]
    fn refine_bpm_keeps_a_half_bpm() {
        let samples = click_track(127.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 127.74, 70, 180), 127.5);
        // The slow half of a drum & bass tempo (175).
        let samples = click_track(87.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 87.37, 70, 180), 87.5);
    }

    #[test]
    fn refine_bpm_corrects_an_estimate_up_to_two_bpm_off() {
        let samples = click_track(128.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 128.2, 70, 180), 128.0);
        assert_eq!(refine_bpm(&samples, 44100, 129.8, 70, 180), 128.0);
    }

    #[test]
    fn refine_bpm_rounds_when_no_tempo_stands_out() {
        // Noise has no beat: every candidate scores alike.
        let mut seed = 1u32;
        let samples = (0..44100 * 60)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect::<Vec<_>>();
        assert_eq!(refine_bpm(&samples, 44100, 127.6, 70, 180), 128.0);
    }

    #[test]
    fn refine_bpm_stays_inside_the_analysis_range() {
        let samples = click_track(181.0, 44100);
        assert!(refine_bpm(&samples, 44100, 179.6, 70, 180) <= 180.0);
    }

    #[test]
    fn refine_bpm_finds_an_off_grid_tempo() {
        // Tracks as long as real ones: the closer a tempo is to a whole or
        // half BPM, the longer the track must be for the difference to show.
        let samples = click_track_secs(174.79, 44100, 300);
        let bpm = refine_bpm(&samples, 44100, 175.3, 70, 180);
        assert!((bpm - 174.79).abs() <= 0.02, "{bpm}");
        let samples = click_track_secs(86.88, 44100, 300);
        let bpm = refine_bpm(&samples, 44100, 87.4, 70, 180);
        assert!((bpm - 86.88).abs() <= 0.02, "{bpm}");
    }

    #[test]
    fn onset_accumulator_matches_the_envelope_however_it_is_fed() {
        let samples = click_track_secs(124.0, 8_000, 20);
        let mut acc = OnsetAccumulator::default();
        let (mut rest, mut size) = (samples.as_slice(), 1usize);
        while !rest.is_empty() {
            let (head, tail) = rest.split_at(size.min(rest.len()));
            acc.push(head);
            rest = tail;
            size = size * 7 % 2_003 + 1;
        }
        assert_eq!(acc.finish(), onset_envelope(&samples));
    }

    /// The early piece is the beat-rich stretch in the first half, the late
    /// one the beat-rich stretch after it; pads alone don't count.
    #[test]
    fn beat_pieces_pick_the_stretches_with_kicks() {
        // 10 minutes at 10 bins a second: a held pad (loud, steady) for the
        // first 2, kicks (sharp rises) from 3 to 5 and from 7 to 9.
        let bins = 6_000;
        let low: Vec<u8> = (0..bins)
            .map(|i| match i {
                0..1_200 => 100,
                1_800..3_000 | 4_200..5_400 if i % 5 == 0 => 120,
                _ => 10,
            })
            .collect();
        let rate = 1_000u32;
        let total = 600 * rate as usize;
        let [early, late] = beat_pieces(&low, total, rate);
        // Within a second: the bins round to the samples they cover.
        let near = |r: &Range<usize>, start: usize| {
            r.start.abs_diff(start * rate as usize) <= rate as usize
                && r.len() == 120 * rate as usize
        };
        assert!(near(&early, 180), "{early:?}");
        assert!(near(&late, 420), "{late:?}");
    }

    #[test]
    fn a_start_and_an_end_stretch_pin_an_off_grid_tempo() {
        // 8 minutes; only the first 3 and the last 2 are looked at.
        let rate = 44100;
        let samples = click_track_secs(174.79, rate, 480);
        let secs = |s: usize| s * rate as usize;
        let envelope = onset_envelope(&samples);
        let onsets = Onsets::from_envelope(&envelope, rate, &[0..secs(180), secs(360)..secs(480)]);
        assert_eq!(onsets.parts.len(), 2);
        let start = Onsets::of(broad_window(&samples, rate), rate);
        let bpm = refine_bpm_onsets(&onsets, &start, 175.3, 70, 180);
        assert!((bpm - 174.79).abs() <= 0.01, "{bpm}");
        // Overlapping stretches are one.
        let merged = Onsets::from_envelope(&envelope, rate, &[0..secs(200), secs(100)..secs(300)]);
        assert_eq!(merged.parts.len(), 1);
    }

    #[test]
    fn first_beat_lands_on_the_first_click() {
        // Clicks every 0.5 s, starting 0.2 s in.
        let mut samples = vec![0f32; 8820];
        samples.extend(click_track(120.0, 44100));
        let first = first_beat_secs(&samples, 44100, 120.0).unwrap();
        assert!((first - 0.2).abs() <= 0.01, "{first}");
    }

    #[test]
    fn refine_bpm_corrects_a_tempo_two_thirds_off() {
        // stratum answering 83 for a 125 track.
        let samples = click_track(125.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 83.2, 70, 180), 125.0);
    }

    #[test]
    fn refine_bpm_uses_the_tempo_that_won_the_family_search() {
        // A raw 85 BPM estimate puts the 3:2 centre at 127.5, but the
        // strongest candidate is 124.5, outside a ±2 search from 127.5.
        let samples = click_track(124.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 85.0, 70, 180), 124.5);
    }

    #[test]
    fn refine_bpm_corrects_a_tempo_five_fourths_off() {
        // A chunk's stratum answering 156.25 for a 125 track, and 100 the
        // other way.
        let samples = click_track(125.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 156.25, 70, 180), 125.0);
        assert_eq!(refine_bpm(&samples, 44100, 100.0, 70, 180), 125.0);
    }

    /// A 128 track with quieter hits on a 160 grid: stratum answering 160
    /// is replaced by 128, which is about twice as coherent; with louder 160
    /// hits (128 only about 1.4 times as coherent) 160 stays.
    #[test]
    fn refine_bpm_steps_down_four_fifths_at_a_moderate_lead() {
        let with_hits_at_160 = |level: f32| -> Vec<f32> {
            click_track(128.0, 44100)
                .iter()
                .zip(click_track(160.0, 44100))
                .map(|(a, b)| a + level * b)
                .collect()
        };
        assert_eq!(
            refine_bpm(&with_hits_at_160(0.3), 44100, 160.0, 70, 180),
            128.0
        );
        assert_eq!(
            refine_bpm(&with_hits_at_160(0.5), 44100, 160.0, 70, 180),
            160.0
        );
    }

    #[test]
    fn refine_bpm_checks_valid_family_tempo_near_range_limit() {
        let samples = click_track(179.0, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 120.5, 70, 180), 179.0);
    }

    #[test]
    fn refine_bpm_recovers_a_strong_tempo_beyond_the_local_search() {
        let samples = click_track(173.71, 44100);
        assert!((refine_bpm(&samples, 44100, 166.5, 70, 180) - 173.71).abs() <= 0.05);
    }

    #[test]
    fn refine_bpm_checks_off_grid_tempo_families() {
        let samples = click_track(158.22, 44100);
        assert!((refine_bpm(&samples, 44100, 105.04, 70, 180) - 158.22).abs() <= 0.05);
    }

    #[test]
    fn refine_bpm_keeps_the_octave_of_the_estimate() {
        let samples = click_track(87.5, 44100);
        assert_eq!(refine_bpm(&samples, 44100, 87.37, 70, 180), 87.5);
        assert_eq!(refine_bpm(&samples, 44100, 175.2, 70, 180), 175.0);
    }
}
