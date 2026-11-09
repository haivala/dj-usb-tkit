//! Project-wide backend logging sink.
//!
//! All backend messages (warnings, errors, dropped rows, panics) are funneled
//! through `emit` so they land in the UI Event Log instead of vanishing into
//! stderr. What only a developer can act on goes through [`debug`] instead: the
//! log file only, in English, never the Event Log. The desktop entry point installs a sink during startup that
//! forwards each message as a `backend:log` Tauri event; tests and CLI
//! binaries leave the sink uninstalled and fall back to stderr.

use std::sync::OnceLock;

/// Severity level for backend messages routed to the Event Log.
#[derive(Debug, Clone, Copy)]
pub enum Level {
    /// Developer diagnostics: the log file only, never the Event Log (see [`debug`]).
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// One log line, as the sink receives it.
#[derive(Debug, Clone, Copy)]
pub struct LogLine<'a> {
    pub level: Level,
    pub source: &'a str,
    /// The warning's code (`usb.import.read-only`), empty for a plain line.
    pub code: &'a str,
    /// What the Event Log shows: the current language for a catalog text.
    pub message: &'a str,
    /// What the log file gets: always English. The same as `message` for text that isn't from
    /// the catalog.
    pub english: &'a str,
}

/// Sink type. The desktop layer installs one of these at startup.
pub type Sink = Box<dyn Fn(&LogLine<'_>) + Send + Sync + 'static>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install a sink. Idempotent on the first call; subsequent calls are
/// ignored (deliberate — there's exactly one process-wide sink).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Emit a log line. Routes to the installed sink when present; falls back
/// to stderr otherwise (CLI tools, tests).
pub fn emit(level: Level, source: &str, message: &str) {
    emit_line(&LogLine {
        level,
        source,
        code: "",
        message,
        english: message,
    });
}

/// Emit a catalog text: the Event Log gets the current language, the log file English.
pub fn emit_text(level: Level, source: &str, text: &crate::i18n::Text) {
    emit_rendered(level, source, "", text);
}

/// Emits both texts and returns the shown one.
fn emit_rendered(level: Level, source: &str, code: &str, text: &crate::i18n::Text) -> String {
    let message = text.render();
    let english = text.render_english();
    emit_line(&LogLine {
        level,
        source,
        code,
        message: &message,
        english: &english,
    });
    message
}

/// Emit a line whose shown and English texts were rendered already (`ApiResponse::failure`).
pub fn emit_line(line: &LogLine<'_>) {
    if let Some(sink) = SINK.get() {
        sink(line);
    } else {
        eprintln!(
            "[{}][{}] {}",
            line.level.as_str(),
            line.source,
            line.english
        );
    }
}

/// A line a command also returns in its `Vec<WarningEntry>` response field, stating
/// `(level, code)` where it's made, for a catalog text: the Event Log and the returned entry
/// get the current language, the log file English. A composed message is one key holding all its values, not a `Text`
/// passed into another (an inner `Text` is rendered when passed, so it wouldn't be English in
/// the file).
pub fn log_text(
    level: Level,
    source: &str,
    code: &str,
    text: &crate::i18n::Text,
) -> crate::models::WarningEntry {
    let message = emit_rendered(level, source, code, text);
    crate::models::WarningEntry {
        level: level.as_str().to_string(),
        code: code.to_string(),
        message,
        source: source.to_string(),
    }
}

/// [`log_text`] for a line rendered twice by its caller: `shown` for the Event Log and the returned
/// entry, `english` for the log file. For a line composed of several catalog texts, each part
/// rendered in the same language; `english` may also add file-format detail the Event Log
/// leaves out.
pub fn log_rendered(
    level: Level,
    source: &str,
    code: &str,
    shown: String,
    english: &str,
) -> crate::models::WarningEntry {
    emit_line(&LogLine {
        level,
        source,
        code,
        message: &shown,
        english,
    });
    crate::models::WarningEntry {
        level: level.as_str().to_string(),
        code: code.to_string(),
        message: shown,
        source: source.to_string(),
    }
}

/// [`log_rendered`] for a line composed of several parts: `compose(false)` is the Event Log's,
/// `compose(true)` the log file's, each part rendered in that one language
/// (`crate::i18n::renderer`, `BackendError::text`).
pub fn log_composed(
    level: Level,
    source: &str,
    code: &str,
    compose: impl Fn(bool) -> String,
) -> crate::models::WarningEntry {
    log_rendered(level, source, code, compose(false), &compose(true))
}

/// How many of `entries` are warnings or errors: the "(N warnings)" a
/// command's status line reports and links to the Event Log.
pub fn count_warnings(entries: &[crate::models::WarningEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.level == Level::Warn.as_str() || entry.level == Level::Error.as_str())
        .count()
}

/// A line only a developer can act on (stage timings, file-format detail, which key opened
/// the eDB): written to the log file, in English, and never shown in the Event Log or
/// returned as a command's warning. Anything a user can act on (fix a file, re-export, run a
/// repair) is catalog text through [`log_text`] instead.
pub fn debug(source: &str, message: impl AsRef<str>) {
    emit(Level::Debug, source, message.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn level_as_str_covers_every_variant() {
        assert_eq!(Level::Debug.as_str(), "debug");
        assert_eq!(Level::Info.as_str(), "info");
        assert_eq!(Level::Warn.as_str(), "warn");
        assert_eq!(Level::Error.as_str(), "error");
    }

    #[test]
    fn log_builds_warning_entry_and_emits_without_panicking() {
        // No sink is guaranteed to be installed in this test binary (installation
        // is process-wide and idempotent -- see `set_sink`), so this also
        // exercises the stderr fallback path in `emit` whenever it runs first.
        let entry = log_rendered(
            Level::Warn,
            "test-source",
            "test.code",
            "hello world".to_string(),
            "hello world (file)",
        );
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.source, "test-source");
        assert_eq!(entry.code, "test.code");
        assert_eq!(entry.message, "hello world");
    }

    #[test]
    fn count_warnings_counts_warn_and_error_entries_only() {
        let entry = |level| log_rendered(level, "src", "code", "line".to_string(), "line");
        let entries = [entry(Level::Info), entry(Level::Warn), entry(Level::Error)];
        assert_eq!(count_warnings(&entries), 2);
        assert_eq!(count_warnings(&[]), 0);
    }

    #[test]
    fn set_sink_routes_emit_through_the_installed_sink() {
        // `SINK` is a process-wide `OnceLock` and `set_sink` is documented as
        // idempotent (deliberately: exactly one sink for the whole process).
        // This is the only call to `set_sink` anywhere in this crate's test
        // suite, so it deterministically wins the race to install it.
        let captured: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_sink = captured.clone();
        set_sink(Box::new(move |line| {
            captured_for_sink.lock().unwrap().push((
                line.level.as_str().to_string(),
                line.source.to_string(),
                line.message.to_string(),
            ));
        }));

        emit(Level::Info, "sink-test-source", "sink test message");

        // Other tests may concurrently emit through this same process-wide
        // sink once it's installed, so assert containment rather than an
        // exact count.
        let entries = captured.lock().unwrap();
        assert!(entries.contains(&(
            "info".to_string(),
            "sink-test-source".to_string(),
            "sink test message".to_string()
        )));
    }
}
