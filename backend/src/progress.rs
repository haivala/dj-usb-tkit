//! Job progress messages with a detail.
//!
//! The progress bar shows a job's message on the right (the step and the
//! file or playlist being worked on, which changes length every moment) and
//! its detail next to the elapsed time on the left: counts, sizes, read
//! speed. The detail rides in the same string as the message, after
//! [`DETAIL_SEPARATOR`], so the `FnMut(usize, usize, &str)` progress callbacks
//! all through the backend carry it unchanged; the job event emitters split
//! it into the payload's own `detail` field ([`split`]).

use crate::i18n::tr;

/// ASCII unit separator: never part of a rendered text.
pub const DETAIL_SEPARATOR: char = '\u{1f}';

/// `message` with `detail` attached.
pub fn with_detail(message: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    format!("{}{DETAIL_SEPARATOR}{}", message.as_ref(), detail.as_ref())
}

/// A progress text's message and detail (`None` without one).
pub fn split(text: &str) -> (&str, Option<&str>) {
    match text.split_once(DETAIL_SEPARATOR) {
        Some((message, detail)) => (message, Some(detail)),
        None => (text, None),
    }
}

/// [`split`], owned: for building a job event payload.
pub fn split_owned(text: String) -> (String, Option<String>) {
    match split(&text) {
        (message, Some(detail)) => (message.to_string(), Some(detail.to_string())),
        _ => (text, None),
    }
}

/// `done/total`, the detail of a step that works through items.
pub fn count(message: impl AsRef<str>, done: usize, total: usize) -> String {
    with_detail(message, tr::progress_detail_count(done, total).render())
}

/// `done/total · 2.6 MB/s`: an item count with the read speed, once measured.
pub fn count_speed(
    message: impl AsRef<str>,
    done: usize,
    total: usize,
    mb_per_s: Option<f64>,
) -> String {
    let detail = match mb_per_s {
        Some(speed) => tr::progress_detail_count_speed(done, total, speed_text(speed)),
        None => tr::progress_detail_count(done, total),
    };
    with_detail(message, detail.render())
}

/// `1.2/1.9 MB · 0.4 MB/s`: bytes copied of a file's size, with the speed
/// once measured.
pub fn bytes_speed(
    message: impl AsRef<str>,
    done: u64,
    total: u64,
    mb_per_s: Option<f64>,
) -> String {
    let mb = |bytes: u64| format!("{:.1}", bytes as f64 / 1_000_000.0);
    let detail = match mb_per_s {
        Some(speed) => tr::progress_detail_bytes_speed(mb(done), mb(total), speed_text(speed)),
        None => tr::progress_detail_bytes(mb(done), mb(total)),
    };
    with_detail(message, detail.render())
}

/// Two decimals below 1 MB/s, so a slow stick doesn't read as 0.0.
fn speed_text(mb_per_s: f64) -> String {
    if mb_per_s < 1.0 {
        format!("{mb_per_s:.2}")
    } else {
        format!("{mb_per_s:.1}")
    }
}

/// Read speed over half-second windows, and when an item that just started
/// is worth showing: at most every 100 ms, so fast media doesn't flood the UI
/// with one event per file.
#[derive(Debug)]
pub struct SpeedMeter {
    window_started: std::time::Instant,
    window_bytes: u64,
    mb_per_s: Option<f64>,
    last_shown: Option<std::time::Instant>,
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeedMeter {
    const WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
    const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

    pub fn new() -> Self {
        Self {
            window_started: std::time::Instant::now(),
            window_bytes: 0,
            mb_per_s: None,
            last_shown: None,
        }
    }

    /// The last measured speed in MB/s.
    pub fn mb_per_s(&self) -> Option<f64> {
        self.mb_per_s
    }

    /// Whether to show the item just started.
    pub fn item_started(&mut self) -> bool {
        let show = self
            .last_shown
            .is_none_or(|at| at.elapsed() >= Self::MIN_INTERVAL);
        if show {
            self.last_shown = Some(std::time::Instant::now());
        }
        show
    }

    /// Counts `bytes` read or written; whether a new speed was measured
    /// (and is worth showing).
    pub fn bytes(&mut self, bytes: u64) -> bool {
        self.window_bytes += bytes;
        let elapsed = self.window_started.elapsed();
        if elapsed < Self::WINDOW {
            return false;
        }
        self.mb_per_s = Some(self.window_bytes as f64 / elapsed.as_secs_f64() / 1_000_000.0);
        self.window_started = std::time::Instant::now();
        self.window_bytes = 0;
        self.last_shown = Some(self.window_started);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detail_splits_back_off_its_message() {
        let text = with_detail("USB: Copying a.mp3", "3/40");
        assert_eq!(split(&text), ("USB: Copying a.mp3", Some("3/40")));
        assert_eq!(split("USB: Saving"), ("USB: Saving", None));
    }

    #[test]
    fn speeds_below_one_mb_keep_two_decimals() {
        assert_eq!(speed_text(0.123), "0.12");
        assert_eq!(speed_text(2.64), "2.6");
    }
}
