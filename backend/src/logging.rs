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

/// Convenience macro: `backend_log!(warn, "pdb-writer", "msg {x}")`.
#[macro_export]
macro_rules! backend_log {
    ($level:ident, $source:expr, $($arg:tt)*) => {{
        let __msg = format!($($arg)*);
        $crate::logging::emit($crate::logging::Level::$level, $source, &__msg);
    }};
}
