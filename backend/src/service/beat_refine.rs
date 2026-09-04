//! First-beat transient-snap refinement.
//!
//! The BPM/key analysis engines (`stratum-dsp`, and the legacy `essentia.js`)
//! report a beat-grid anchor derived from onset detection running on
//! coarse STFT frames (stratum-dsp's default is a 2048-sample window with a
//! 512-sample hop, ~11.6ms per hop at 44.1kHz), and onset-flux detectors
//! systematically lag the true transient peak by roughly a frame while the
//! flux rises past threshold. Worse, a track with a quiet/silent intro can
//! make the engine's beat tracker lock onto a spurious near-zero-time onset
//! (an STFT boundary artifact at the very start of the buffer) that has
//! nothing to do with the track's real first hit -- confirmed empirically
//! against `stratum-dsp` (see the functional test in `analysis.rs`).
//!
//! This module re-examines the raw decoded audio (already resident in memory
//! from analysis) to correct both failure modes, using a cheap time-domain
//! onset-strength function at much finer resolution than the engines' own
//! STFT hop:
//!
//! 1. **Local snap**: search a small window around the engine's estimate for
//!    the true attack transient and snap to it. Fixes ordinary hop
//!    quantization/attack-detection lag.
//! 2. **Silent-window rescue**: if that window turns out to be genuine
//!    silence (not just "quiet with no sharp attack"), the engine's anchor is
//!    almost certainly a false lock rather than a real, if soft, onset.
//!    Scan forward for the track's actual first onset and re-run the local
//!    snap around that instead.
//!
//! Deliberately not a general onset detector: bounded windows, no
//! allocation-heavy STFT, single pass. Engine-agnostic by construction (it
//! only sees `(samples, sample_rate, bpm, first_beat_ms)`), so it applies
//! uniformly regardless of which engine produced the estimate.

/// Widest local-snap window we'll ever search, regardless of BPM.
const MAX_SEARCH_WINDOW_MS: f64 = 60.0;
/// Local-snap window as a fraction of one beat interval, before clamping.
const MAX_SEARCH_WINDOW_BEAT_FRACTION: f64 = 0.20;
/// Narrowest local-snap window we'll search -- keeps enough sub-frames for
/// stable stats even at very high BPM.
const MIN_SEARCH_WINDOW_MS: f64 = 16.0;
/// Onset-strength sub-frame size for the local snap. Far finer than
/// stratum-dsp's ~11.6ms STFT hop, which is the whole point -- it lets us
/// snap phase within the coarse beat the engine already found.
const SUBFRAME_MS: f64 = 2.0;
/// A candidate transient's rise must account for at least this fraction of
/// the window's peak energy -- a true attack jumps most of the way to full
/// amplitude within a single sub-frame, whereas a gradual swell/riser
/// spreads the same total rise thinly across many sub-frames. Rejects
/// ambient/pad intros with no clean attack instead of snapping to noise.
const MIN_RISE_FRACTION_OF_PEAK_ENERGY: f32 = 0.25;
/// Absolute floor on a window's peak energy, guarding near-silent windows
/// where any rise is just quantization/rounding noise.
const MIN_ABSOLUTE_ENERGY: f32 = 1e-4;
/// Coarser sub-frame size used only for the long-range rescue scan below --
/// cheap since it only runs on the (rare) silent-window path.
const RESCUE_SUBFRAME_MS: f64 = 5.0;
/// How far forward we're willing to scan for the track's real first onset
/// when rescuing from a silent-window false lock.
const RESCUE_MAX_SCAN_SECS: f64 = 20.0;
/// A rescue candidate's rise must clear this multiple of the leading noise
/// floor to count as a genuine onset rather than dither/rounding noise.
const RESCUE_ONSET_FLOOR_MULTIPLE: f32 = 8.0;

/// Refine an engine-reported first-beat time against the raw decoded audio.
/// Always returns a usable value: falls back to `reported_first_beat_ms`
/// unchanged whenever no confident correction is found, or the inputs are
/// too degenerate to search (empty buffer, zero sample rate).
pub(crate) fn refine_first_beat_ms(
    samples: &[f32],
    sample_rate: u32,
    bpm: Option<f64>,
    reported_first_beat_ms: u32,
) -> u32 {
    let window_ms = search_window_ms(bpm);
    if let Some(snapped) =
        find_transient_snap_ms(samples, sample_rate, window_ms, reported_first_beat_ms)
    {
        return snapped;
    }
    // The local window came back empty. If that's because the reported time
    // sits inside genuine leading silence -- as opposed to quiet-but-present
    // content with no sharp attack -- the engine likely locked onto a
    // spurious near-zero onset rather than the track's real first hit.
    // Rescue by scanning further out for where the track actually starts,
    // then re-run the tight local snap around that.
    if window_is_silent(samples, sample_rate, window_ms, reported_first_beat_ms)
        && let Some(rescued_ms) = find_first_onset_ms(samples, sample_rate, reported_first_beat_ms)
    {
        return find_transient_snap_ms(samples, sample_rate, window_ms, rescued_ms)
            .unwrap_or(rescued_ms);
    }
    reported_first_beat_ms
}

/// `min(60ms, 20% of one beat interval)`, floored at 16ms. Missing/
/// non-positive BPM falls back to the flat 60ms cap.
fn search_window_ms(bpm: Option<f64>) -> f64 {
    match bpm.filter(|b| *b > 0.0) {
        Some(bpm) => {
            let beat_interval_ms = 60_000.0 / bpm;
            (beat_interval_ms * MAX_SEARCH_WINDOW_BEAT_FRACTION)
                .clamp(MIN_SEARCH_WINDOW_MS, MAX_SEARCH_WINDOW_MS)
        }
        None => MAX_SEARCH_WINDOW_MS,
    }
}

/// A rectified-energy envelope over a bounded span of `samples`, in
/// `subframe_len`-sample non-overlapping sub-frames starting at
/// `start_sample`.
struct EnergyWindow {
    start_sample: usize,
    subframe_len: usize,
    energy: Vec<f32>,
}

/// Build the energy envelope for a `window_ms`-wide span centered on
/// `center_ms`. `None` if the buffer is empty/degenerate or the window
/// doesn't fit at least a handful of sub-frames.
fn compute_energy_window(
    samples: &[f32],
    sample_rate: u32,
    window_ms: f64,
    center_ms: u32,
) -> Option<EnergyWindow> {
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }
    let sample_rate_f = sample_rate as f64;
    let subframe_len = ((SUBFRAME_MS / 1000.0) * sample_rate_f).round().max(1.0) as usize;

    let center = ((center_ms as f64 / 1000.0) * sample_rate_f).round() as i64;
    let half_window_samples = ((window_ms / 1000.0) * sample_rate_f).round().max(1.0) as i64;
    let start = center.saturating_sub(half_window_samples).max(0) as usize;
    let end = ((center.saturating_add(half_window_samples)).max(0) as usize).min(samples.len());
    if end <= start {
        return None;
    }
    let window = &samples[start..end];
    // Need a handful of sub-frames for the salience stats to mean anything.
    if window.len() < subframe_len * 4 {
        return None;
    }

    let energy = window
        .chunks(subframe_len)
        .map(|chunk| chunk.iter().map(|s| s.abs()).sum::<f32>() / chunk.len() as f32)
        .collect();
    Some(EnergyWindow {
        start_sample: start,
        subframe_len,
        energy,
    })
}

/// Whether the `window_ms`-wide span centered on `center_ms` is genuine
/// silence (as opposed to quiet-but-present content). `false` if a window
/// couldn't even be built -- that's "unknown", not "silent".
fn window_is_silent(samples: &[f32], sample_rate: u32, window_ms: f64, center_ms: u32) -> bool {
    match compute_energy_window(samples, sample_rate, window_ms, center_ms) {
        Some(w) => w.energy.iter().cloned().fold(0.0f32, f32::max) < MIN_ABSOLUTE_ENERGY,
        None => false,
    }
}

/// Search a `window_ms`-wide span centered on `reported_first_beat_ms` for
/// the sharpest attack transient, returning its refined ms position only if
/// it clears the confidence guard.
fn find_transient_snap_ms(
    samples: &[f32],
    sample_rate: u32,
    window_ms: f64,
    reported_first_beat_ms: u32,
) -> Option<u32> {
    let w = compute_energy_window(samples, sample_rate, window_ms, reported_first_beat_ms)?;
    if w.energy.len() < 2 {
        return None;
    }

    let max_energy = w.energy.iter().cloned().fold(0.0f32, f32::max);
    if max_energy < MIN_ABSOLUTE_ENERGY {
        return None;
    }

    // flux[i] is the rise entering energy-frame i, relative to frame i-1.
    // There's no frame -1 to compare frame 0 against, so frame 0 is never a
    // candidate -- that boundary has no defined rise, not a rise of zero.
    let mut best_frame_idx = None;
    let mut best_flux = 0.0f32;
    for i in 1..w.energy.len() {
        let flux = (w.energy[i] - w.energy[i - 1]).max(0.0);
        if flux > best_flux {
            best_flux = flux;
            best_frame_idx = Some(i);
        }
    }
    let best_frame_idx = best_frame_idx?;

    if best_flux < MIN_RISE_FRACTION_OF_PEAK_ENERGY * max_energy {
        return None;
    }

    // Snap to the leading edge of the winning sub-frame, not its center --
    // this is what corrects the systematic "flux lags the true attack"
    // latency of frame-based onset detectors.
    let snap_sample = w.start_sample + best_frame_idx * w.subframe_len;
    Some(((snap_sample as f64 / sample_rate as f64) * 1000.0).round() as u32)
}

/// Scan forward from `from_ms` across up to `RESCUE_MAX_SCAN_SECS` of audio
/// for the first sub-frame that clearly rises above the leading noise floor
/// -- i.e. the track's real first onset, used when the engine's own
/// estimate landed inside genuine leading silence.
fn find_first_onset_ms(samples: &[f32], sample_rate: u32, from_ms: u32) -> Option<u32> {
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }
    let sample_rate_f = sample_rate as f64;
    let subframe_len = ((RESCUE_SUBFRAME_MS / 1000.0) * sample_rate_f)
        .round()
        .max(1.0) as usize;
    let start =
        (((from_ms as f64 / 1000.0) * sample_rate_f).round().max(0.0) as usize).min(samples.len());
    if samples.len() - start < subframe_len * 4 {
        return None;
    }
    let scan_len = ((RESCUE_MAX_SCAN_SECS * sample_rate_f) as usize).min(samples.len() - start);
    let window = &samples[start..start + scan_len];

    let energy: Vec<f32> = window
        .chunks(subframe_len)
        .map(|chunk| chunk.iter().map(|s| s.abs()).sum::<f32>() / chunk.len() as f32)
        .collect();
    if energy.len() < 2 {
        return None;
    }

    // Leading noise floor: this path is only taken because the caller
    // already established the scan starts in silence, so the quietest
    // sub-frame here is a good stand-in for "nothing," and a clear multiple
    // above it is a real onset rather than dither/rounding noise.
    let floor = energy
        .iter()
        .cloned()
        .fold(f32::MAX, f32::min)
        .max(MIN_ABSOLUTE_ENERGY);
    let threshold = floor * RESCUE_ONSET_FLOOR_MULTIPLE;
    let idx = energy.iter().position(|&e| e >= threshold)?;

    let snap_sample = start + idx * subframe_len;
    Some(((snap_sample as f64 / sample_rate_f) * 1000.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    /// A buffer of `len_samples` silence with a short linear-ramp impulse of
    /// `amplitude` inserted starting at `at_sample`.
    fn click_track(len_samples: usize, at_sample: usize, amplitude: f32) -> Vec<f32> {
        let mut samples = vec![0.0f32; len_samples];
        // A few samples of sharp attack followed by a short decay, so the
        // energy envelope actually has a clean rise (a single-sample delta
        // would mostly vanish under sub-frame averaging).
        let attack = [0.3, 0.7, 1.0, 0.9, 0.6, 0.3, 0.1];
        for (i, &a) in attack.iter().enumerate() {
            if let Some(s) = samples.get_mut(at_sample + i) {
                *s = amplitude * a;
            }
        }
        samples
    }

    fn ms_to_samples(ms: f64) -> usize {
        ((ms / 1000.0) * SR as f64).round() as usize
    }

    fn samples_to_ms(samples: usize) -> u32 {
        ((samples as f64 / SR as f64) * 1000.0).round() as u32
    }

    #[test]
    fn snaps_true_click_to_itself() {
        let at = ms_to_samples(2267.0);
        let samples = click_track(SR as usize * 5, at, 1.0);
        let true_ms = samples_to_ms(at);
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), true_ms);
        assert!(
            refined.abs_diff(true_ms) <= 3,
            "refined={refined} true={true_ms}"
        );
    }

    #[test]
    fn corrects_late_reported_estimate() {
        let at = ms_to_samples(2267.0);
        let samples = click_track(SR as usize * 5, at, 1.0);
        let true_ms = samples_to_ms(at);
        let reported = true_ms + 25;
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), reported);
        assert!(
            refined.abs_diff(true_ms) <= 3,
            "refined={refined} true={true_ms}"
        );
        assert!(refined.abs_diff(true_ms) < reported.abs_diff(true_ms));
    }

    #[test]
    fn corrects_early_reported_estimate() {
        let at = ms_to_samples(2267.0);
        let samples = click_track(SR as usize * 5, at, 1.0);
        let true_ms = samples_to_ms(at);
        let reported = true_ms.saturating_sub(20);
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), reported);
        assert!(
            refined.abs_diff(true_ms) <= 3,
            "refined={refined} true={true_ms}"
        );
        assert!(refined.abs_diff(true_ms) < reported.abs_diff(true_ms));
    }

    #[test]
    fn silence_leaves_estimate_unchanged() {
        let samples = vec![0.0f32; SR as usize * 5];
        let reported = 2267u32;
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), reported);
        assert_eq!(refined, reported);
    }

    #[test]
    fn gradual_swell_leaves_estimate_unchanged() {
        // A slow linear ramp across the whole search window -- no sharp
        // attack, so the rise is spread thin rather than concentrated.
        let mut samples = vec![0.0f32; SR as usize * 5];
        let reported = 2267u32;
        let center = ms_to_samples(reported as f64);
        let window = ms_to_samples(60.0);
        let start = center.saturating_sub(window);
        let end = (center + window).min(samples.len());
        for (i, s) in samples[start..end].iter_mut().enumerate() {
            *s = 0.05 * (i as f32 / (end - start) as f32);
        }
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), reported);
        assert_eq!(refined, reported);
    }

    #[test]
    fn snaps_to_stronger_of_two_clicks() {
        let weak_at = ms_to_samples(2250.0);
        let strong_at = ms_to_samples(2275.0);
        let mut samples = click_track(SR as usize * 5, weak_at, 0.1);
        for (i, v) in click_track(SR as usize * 5, strong_at, 1.0)
            .into_iter()
            .enumerate()
        {
            samples[i] += v;
        }
        let true_strong_ms = samples_to_ms(strong_at);
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), 2260);
        assert!(
            refined.abs_diff(true_strong_ms) <= 3,
            "refined={refined} true_strong={true_strong_ms}"
        );
    }

    #[test]
    fn empty_samples_returns_input_unchanged() {
        assert_eq!(refine_first_beat_ms(&[], SR, Some(128.0), 500), 500);
    }

    #[test]
    fn zero_sample_rate_returns_input_unchanged() {
        let samples = click_track(SR as usize, 1000, 1.0);
        assert_eq!(refine_first_beat_ms(&samples, 0, Some(128.0), 500), 500);
    }

    #[test]
    fn window_near_buffer_start_does_not_panic() {
        let samples = click_track(SR as usize, 5, 1.0);
        // reported_first_beat_ms near 0 -- window would want to go negative.
        let _ = refine_first_beat_ms(&samples, SR, Some(128.0), 0);
    }

    #[test]
    fn window_near_buffer_end_does_not_panic() {
        let len = SR as usize;
        let samples = click_track(len, len - 5, 1.0);
        let near_end_ms = samples_to_ms(len - 1);
        let _ = refine_first_beat_ms(&samples, SR, Some(128.0), near_end_ms + 100);
    }

    #[test]
    fn search_window_ms_flat_cap_when_bpm_missing() {
        assert_eq!(search_window_ms(None), MAX_SEARCH_WINDOW_MS);
    }

    #[test]
    fn search_window_ms_low_bpm_clamps_to_max() {
        // interval 1000ms, 20% = 200ms -> clamped down to 60ms.
        assert_eq!(search_window_ms(Some(60.0)), MAX_SEARCH_WINDOW_MS);
    }

    #[test]
    fn search_window_ms_mid_bpm_uses_fraction() {
        // interval 300ms, 20% = 60ms -- right at the cap boundary.
        assert_eq!(search_window_ms(Some(200.0)), 60.0);
        // interval 100ms, 20% = 20ms.
        assert!((search_window_ms(Some(600.0)) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn search_window_ms_high_bpm_clamps_to_min() {
        // interval 20ms, 20% = 4ms -> clamped up to the 16ms floor.
        assert_eq!(search_window_ms(Some(3000.0)), MIN_SEARCH_WINDOW_MS);
    }

    #[test]
    fn search_window_ms_non_positive_bpm_falls_back_to_flat_cap() {
        assert_eq!(search_window_ms(Some(0.0)), MAX_SEARCH_WINDOW_MS);
        assert_eq!(search_window_ms(Some(-10.0)), MAX_SEARCH_WINDOW_MS);
    }

    #[test]
    fn rescues_true_onset_after_leading_silence() {
        // 3 seconds of silence, then a click -- simulates a track with a
        // quiet intro where the engine's beat tracker locked onto a
        // near-zero false onset (the reported estimate below).
        let at = ms_to_samples(3000.0);
        let samples = click_track(SR as usize * 6, at, 1.0);
        let true_ms = samples_to_ms(at);
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), 12);
        assert!(
            refined.abs_diff(true_ms) <= 6,
            "refined={refined} true={true_ms}"
        );
    }

    #[test]
    fn rescue_scan_finds_nothing_leaves_estimate_unchanged() {
        // Silence everywhere, including well beyond the rescue scan's
        // horizon -- there's nothing to rescue into, so the original
        // (already-empty) estimate should come back unchanged.
        let samples = vec![0.0f32; SR as usize * 25];
        let refined = refine_first_beat_ms(&samples, SR, Some(128.0), 12);
        assert_eq!(refined, 12);
    }
}
