//! Metronome clicks mixed into the playing track by the native engine.
//!
//! The cue editor's metronome clicks on the beat grid while a track plays, so
//! the user can hear whether the grid lines up. The clicks are added to the
//! decoded samples themselves ([`MetronomeSource`] wraps the track source), so
//! they sit sample-accurately on the grid of what is actually heard, follow
//! seeks, and need no second audio path (the webview's audio may be missing
//! entirely, e.g. WebKitGTK without GStreamer's `autoaudiosink`).
//!
//! [`MetronomeSettings`] is shared between the controller (which the frontend
//! updates while a track plays) and the audio thread, as lock-free atomics.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::Source;

/// How long one click rings out.
const CLICK_MS: f64 = 30.0;
/// Decay time constant of the click envelope.
const CLICK_DECAY_MS: f64 = 6.0;
/// Bar starts (every 4th beat from the first) click higher and louder.
const DOWNBEAT_HZ: f64 = 1600.0;
const BEAT_HZ: f64 = 1000.0;
const DOWNBEAT_GAIN: f64 = 0.45;
const BEAT_GAIN: f64 = 0.3;

/// The metronome's on/off switch and beat grid.
#[derive(Debug, Default)]
pub struct MetronomeSettings {
    enabled: AtomicBool,
    /// f64 bits.
    first_beat_ms: AtomicU64,
    /// f64 bits; 0 when there is no usable BPM.
    beat_interval_ms: AtomicU64,
}

impl MetronomeSettings {
    /// Returns whether the metronome is now on: it stays off without a usable
    /// grid (a non-finite or non-positive BPM).
    pub fn set(&self, enabled: bool, first_beat_ms: f64, bpm: f64) -> bool {
        let interval = if bpm.is_finite() && bpm > 0.0 { 60_000.0 / bpm } else { 0.0 };
        let first_beat = if first_beat_ms.is_finite() { first_beat_ms.max(0.0) } else { 0.0 };
        self.first_beat_ms.store(first_beat.to_bits(), Ordering::Relaxed);
        self.beat_interval_ms.store(interval.to_bits(), Ordering::Relaxed);
        let on = enabled && interval > 0.0;
        self.enabled.store(on, Ordering::Relaxed);
        on
    }

    /// `(first_beat_ms, beat_interval_ms)` while on.
    fn grid(&self) -> Option<(f64, f64)> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        let interval = f64::from_bits(self.beat_interval_ms.load(Ordering::Relaxed));
        (interval > 0.0).then(|| {
            (f64::from_bits(self.first_beat_ms.load(Ordering::Relaxed)), interval)
        })
    }
}

/// The click sample (as a fraction of full scale) at track time `t_ms`.
fn click_at(grid: Option<(f64, f64)>, t_ms: f64) -> f64 {
    let Some((first_beat, interval)) = grid else {
        return 0.0;
    };
    if t_ms < first_beat {
        return 0.0;
    }
    let beat = ((t_ms - first_beat) / interval).floor();
    let dt = t_ms - (first_beat + beat * interval);
    if dt >= CLICK_MS {
        return 0.0;
    }
    let downbeat = (beat as u64).is_multiple_of(4);
    let (hz, gain) = if downbeat { (DOWNBEAT_HZ, DOWNBEAT_GAIN) } else { (BEAT_HZ, BEAT_GAIN) };
    gain * (-dt / CLICK_DECAY_MS).exp() * (TAU * hz * dt / 1000.0).sin()
}

/// Wraps the track source and adds a click on each grid beat while the
/// metronome is on. Tracks its own position in the track (starting at the
/// offset playback began from, and reset by seeks), so a click lands where the
/// beat is in the audio, whatever the output latency.
pub struct MetronomeSource<S> {
    inner: S,
    settings: Arc<MetronomeSettings>,
    /// Track time of the next frame.
    position_ms: f64,
    sample_in_frame: u16,
    channels: u16,
    /// The click for the current frame, the same on every channel.
    click: i16,
}

impl<S: Source<Item = i16>> MetronomeSource<S> {
    pub fn new(inner: S, settings: Arc<MetronomeSettings>, start_ms: u64) -> Self {
        Self {
            inner,
            settings,
            position_ms: start_ms as f64,
            sample_in_frame: 0,
            channels: 1,
            click: 0,
        }
    }
}

impl<S: Source<Item = i16>> Iterator for MetronomeSource<S> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let sample = self.inner.next()?;
        if self.sample_in_frame == 0 {
            self.channels = self.inner.channels().max(1);
            let rate = f64::from(self.inner.sample_rate().max(1));
            let click = click_at(self.settings.grid(), self.position_ms);
            self.click = (click * f64::from(i16::MAX)).round() as i16;
            self.position_ms += 1000.0 / rate;
        }
        self.sample_in_frame += 1;
        if self.sample_in_frame >= self.channels {
            self.sample_in_frame = 0;
        }
        Some(sample.saturating_add(self.click))
    }
}

impl<S: Source<Item = i16>> Source for MetronomeSource<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        self.position_ms = pos.as_secs_f64() * 1000.0;
        self.sample_in_frame = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::source::Zero;

    const RATE: u32 = 48_000;

    /// Mono silence through the metronome: the samples are the clicks alone.
    fn clicks(settings: &Arc<MetronomeSettings>, start_ms: u64, seconds: f64) -> Vec<i16> {
        let silence = Zero::<i16>::new(1, RATE);
        MetronomeSource::new(silence, settings.clone(), start_ms)
            .take((seconds * f64::from(RATE)) as usize)
            .collect()
    }

    /// Peak level in the window [from_ms, to_ms) of a buffer that starts at `start_ms`.
    fn peak(samples: &[i16], start_ms: f64, from_ms: f64, to_ms: f64) -> i16 {
        let at = |ms: f64| (((ms - start_ms) / 1000.0) * f64::from(RATE)).max(0.0) as usize;
        samples[at(from_ms).min(samples.len())..at(to_ms).min(samples.len())]
            .iter()
            .map(|s| s.saturating_abs())
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn off_by_default_and_without_a_usable_bpm() {
        let settings = Arc::new(MetronomeSettings::default());
        assert!(clicks(&settings, 0, 1.0).iter().all(|s| *s == 0));
        assert!(!settings.set(true, 0.0, 0.0));
        assert!(!settings.set(true, 0.0, f64::NAN));
        assert!(clicks(&settings, 0, 1.0).iter().all(|s| *s == 0));
    }

    #[test]
    fn clicks_on_each_beat_from_the_first_beat_and_is_silent_between() {
        let settings = Arc::new(MetronomeSettings::default());
        // 120 BPM from 100 ms: beats at 100, 600, 1100, 1600 ms.
        assert!(settings.set(true, 100.0, 120.0));
        let samples = clicks(&settings, 0, 2.0);
        assert_eq!(peak(&samples, 0.0, 0.0, 99.0), 0, "nothing before the first beat");
        for beat in [100.0, 600.0, 1100.0, 1600.0] {
            assert!(peak(&samples, 0.0, beat, beat + 10.0) > 3000, "click at {beat} ms");
            assert_eq!(peak(&samples, 0.0, beat + 40.0, beat + 450.0), 0, "silent after {beat} ms");
        }
    }

    #[test]
    fn bar_starts_are_louder() {
        let settings = Arc::new(MetronomeSettings::default());
        settings.set(true, 0.0, 120.0);
        let samples = clicks(&settings, 0, 2.5);
        let downbeat = peak(&samples, 0.0, 0.0, 30.0);
        let beat = peak(&samples, 0.0, 500.0, 530.0);
        assert!(downbeat > beat, "downbeat {downbeat} vs beat {beat}");
        assert!(peak(&samples, 0.0, 2000.0, 2030.0) > beat, "beat 4 starts the next bar");
    }

    #[test]
    fn starts_from_the_playback_offset_and_follows_seeks() {
        let settings = Arc::new(MetronomeSettings::default());
        settings.set(true, 0.0, 120.0);
        // Playback began at 10.25 s: the next beat is at 10.5 s (0.25 s in).
        let samples = clicks(&settings, 10_250, 0.5);
        assert_eq!(peak(&samples, 10_250.0, 10_250.0, 10_490.0), 0);
        assert!(peak(&samples, 10_250.0, 10_500.0, 10_510.0) > 3000);

        let mut source = MetronomeSource::new(Zero::<i16>::new(1, RATE), settings, 0);
        source.try_seek(Duration::from_millis(30_100)).expect("zero source seeks");
        // 30.1 s is 100 ms past a beat: silent until the 30.5 s beat.
        let after_seek: Vec<i16> = source.by_ref().take((RATE as usize) * 3 / 10).collect();
        assert!(after_seek.iter().all(|s| *s == 0));
        // 30.4–30.55 s: takes in the 30.5 s beat.
        let next: Vec<i16> = source.take((RATE as usize) * 15 / 100).collect();
        assert!(next.iter().any(|s| s.saturating_abs() > 3000));
    }

    #[test]
    fn toggles_live_while_playing() {
        let settings = Arc::new(MetronomeSettings::default());
        let mut source = MetronomeSource::new(Zero::<i16>::new(1, RATE), settings.clone(), 0);
        let first: Vec<i16> = source.by_ref().take(RATE as usize).collect();
        assert!(first.iter().all(|s| *s == 0));
        settings.set(true, 0.0, 120.0);
        let second: Vec<i16> = source.take(RATE as usize).collect();
        assert!(second.iter().any(|s| s.saturating_abs() > 3000));
    }

    #[test]
    fn same_click_on_every_channel_of_a_frame() {
        let settings = Arc::new(MetronomeSettings::default());
        settings.set(true, 0.0, 120.0);
        let stereo: Vec<i16> = MetronomeSource::new(Zero::<i16>::new(2, RATE), settings, 0)
            .take(2 * 480)
            .collect();
        for frame in stereo.chunks(2) {
            assert_eq!(frame[0], frame[1]);
        }
        assert!(stereo.iter().any(|s| *s != 0));
    }
}
