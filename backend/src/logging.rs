//! Project-wide backend logging sink.
//!
//! All backend messages (warnings, errors, dropped rows, panics) are funneled
//! through `emit` so they land in the UI Event Log instead of vanishing into
//! stderr. The desktop entry point installs a sink during startup that
//! forwards each message as a `backend:log` Tauri event; tests and CLI
//! binaries leave the sink uninstalled and fall back to stderr.

use std::sync::OnceLock;

/// Severity level for backend messages routed to the Event Log.
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Sink type. The desktop layer installs one of these at startup.
pub type Sink = Box<dyn Fn(Level, &str, &str) + Send + Sync + 'static>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install a sink. Idempotent on the first call; subsequent calls are
/// ignored (deliberate — there's exactly one process-wide sink).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Emit a log line. Routes to the installed sink when present; falls back
/// to stderr otherwise (CLI tools, tests).
pub fn emit(level: Level, source: &str, message: &str) {
    if let Some(sink) = SINK.get() {
        sink(level, source, message);
    } else {
        eprintln!("[{}][{}] {}", level.as_str(), source, message);
    }
}

/// The one way to produce a log line that a command also wants to return in
/// its own `Vec<WarningEntry>` response field: state `(level, code)`
/// explicitly at the point the message is created, emit it live via
/// `emit()`, and get back the `WarningEntry` to push. Replaces the old
/// pattern of building a bare `String` and re-classifying it later by
/// guessing from substrings.
pub fn log(
    level: Level,
    source: &str,
    code: &str,
    message: impl Into<String>,
) -> crate::models::WarningEntry {
    let message = message.into();
    emit(level, source, &message);
    crate::models::WarningEntry {
        level: level.as_str().to_string(),
        code: code.to_string(),
        message,
        source: source.to_string(),
    }
}

/// Convenience macro: `backend_log!(warn, "pdb-writer", "msg {x}")`.
#[macro_export]
macro_rules! backend_log {
    ($level:ident, $source:expr, $($arg:tt)*) => {{
        let __msg = format!($($arg)*);
        $crate::logging::emit($crate::logging::Level::$level, $source, &__msg);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn level_as_str_covers_every_variant() {
        assert_eq!(Level::Info.as_str(), "info");
        assert_eq!(Level::Warn.as_str(), "warn");
        assert_eq!(Level::Error.as_str(), "error");
    }

    #[test]
    fn log_builds_warning_entry_and_emits_without_panicking() {
        // No sink is guaranteed to be installed in this test binary (installation
        // is process-wide and idempotent -- see `set_sink`), so this also
        // exercises the stderr fallback path in `emit` whenever it runs first.
        let entry = log(Level::Warn, "test-source", "test.code", "hello world");
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.source, "test-source");
        assert_eq!(entry.code, "test.code");
        assert_eq!(entry.message, "hello world");
    }

    #[test]
    fn log_accepts_owned_string_and_str_messages() {
        let from_str = log(Level::Info, "src", "code", "static message");
        assert_eq!(from_str.message, "static message");

        let owned = format!("dynamic {}", 42);
        let from_string = log(Level::Error, "src", "code", owned.clone());
        assert_eq!(from_string.message, owned);
    }

    #[test]
    fn set_sink_routes_emit_through_the_installed_sink() {
        // `SINK` is a process-wide `OnceLock` and `set_sink` is documented as
        // idempotent (deliberately: exactly one sink for the whole process).
        // This is the only call to `set_sink` anywhere in this crate's test
        // suite, so it deterministically wins the race to install it.
        let captured: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_sink = captured.clone();
        set_sink(Box::new(move |level, source, message| {
            captured_for_sink.lock().unwrap().push((
                level.as_str().to_string(),
                source.to_string(),
                message.to_string(),
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
