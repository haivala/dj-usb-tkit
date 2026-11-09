use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::i18n::Text;

#[derive(Debug, Error)]
pub enum BackendError {
    /// Something the user can act on, from the text catalog: shown in the current language,
    /// logged in English. A bug's error is `Internal`, in English.
    #[error("validation error: {}", .0.render_english())]
    Validation(Text),
    #[error("validation error: {}", .0.render_english())]
    ValidationWithDetails(Text, Value),
    /// Something the user asked for that isn't there; see `Validation`.
    #[error("not found: {}", .0.render_english())]
    NotFound(Text),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    ValidationError,
    NotFound,
    IoError,
    DbError,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::ValidationError => "error.validation",
            ErrorCode::NotFound => "error.not_found",
            ErrorCode::IoError => "error.io",
            ErrorCode::DbError => "error.db",
            ErrorCode::InternalError => "error.internal",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: ErrorCode,
    /// In the current language: what the user sees.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// The message in English for the log file, when it differs from `message` (a catalog
    /// text). `None` means `message` is already what the log gets.
    #[serde(skip)]
    pub log_message: Option<String>,
}

impl ErrorPayload {
    /// A payload whose message is the same in the response and the log.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            log_message: None,
        }
    }

    fn from_text(code: ErrorCode, text: &Text) -> Self {
        Self {
            code,
            message: text.render(),
            details: None,
            log_message: Some(text.render_english()),
        }
    }

    /// The message the log file gets: English.
    pub fn log_text(&self) -> &str {
        self.log_message.as_deref().unwrap_or(&self.message)
    }
}

impl From<BackendError> for ErrorPayload {
    fn from(value: BackendError) -> Self {
        match value {
            BackendError::Validation(text) => Self::from_text(ErrorCode::ValidationError, &text),
            BackendError::ValidationWithDetails(text, details) => Self {
                details: Some(details),
                ..Self::from_text(ErrorCode::ValidationError, &text)
            },
            BackendError::NotFound(text) => Self::from_text(ErrorCode::NotFound, &text),
            BackendError::Io(err) => Self::new(ErrorCode::IoError, err.to_string()),
            BackendError::Db(_err) => {
                // Release builds keep the SQLite detail out of the returned
                // message, so put it in the Event Log -- otherwise it's lost.
                crate::logging::emit(
                    crate::logging::Level::Error,
                    "database",
                    &format!("database failure: {_err}"),
                );
                #[cfg(debug_assertions)]
                let message = format!("database failure: {_err}");
                #[cfg(not(debug_assertions))]
                let message = "an internal database error occurred".to_string();
                Self::new(ErrorCode::DbError, message)
            }
            BackendError::Internal(msg) => Self::new(ErrorCode::InternalError, msg),
        }
    }
}

impl BackendError {
    /// What a user reads: a catalog text in the current language, without the English kind
    /// prefix `Display` adds; any other error as `Display` has it.
    pub fn render(&self) -> String {
        self.text(false)
    }

    /// [`Self::render`], or with `english` the log file's text (`Display`, prefix included).
    pub fn text(&self, english: bool) -> String {
        match self {
            _ if english => self.to_string(),
            BackendError::Validation(text)
            | BackendError::ValidationWithDetails(text, _)
            | BackendError::NotFound(text) => text.render(),
            other => other.to_string(),
        }
    }
}

pub type BackendResult<T> = Result<T, BackendError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_as_str_covers_every_variant() {
        assert_eq!(ErrorCode::ValidationError.as_str(), "error.validation");
        assert_eq!(ErrorCode::NotFound.as_str(), "error.not_found");
        assert_eq!(ErrorCode::IoError.as_str(), "error.io");
        assert_eq!(ErrorCode::DbError.as_str(), "error.db");
        assert_eq!(ErrorCode::InternalError.as_str(), "error.internal");
    }

    #[test]
    fn error_payload_from_validation_error() {
        let payload: ErrorPayload =
            BackendError::Validation(crate::i18n::tr::playlist_error_name_empty()).into();
        assert!(matches!(payload.code, ErrorCode::ValidationError));
        assert_eq!(payload.message, "playlist name must not be empty");
        assert!(payload.details.is_none());
    }

    #[test]
    fn error_payload_from_validation_with_details() {
        let details = serde_json::json!({"field": "x"});
        let payload: ErrorPayload = BackendError::ValidationWithDetails(
            crate::i18n::tr::playlist_error_name_empty(),
            details.clone(),
        )
        .into();
        assert!(matches!(payload.code, ErrorCode::ValidationError));
        assert_eq!(payload.details, Some(details));
    }

    #[test]
    fn error_payload_from_not_found() {
        let payload: ErrorPayload =
            BackendError::NotFound(crate::i18n::tr::error_track_not_found("t1")).into();
        assert!(matches!(payload.code, ErrorCode::NotFound));
        assert_eq!(payload.message, "track not found: t1");
    }

    #[test]
    fn error_payload_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let payload: ErrorPayload = BackendError::Io(io_err).into();
        assert!(matches!(payload.code, ErrorCode::IoError));
        assert!(payload.message.contains("file missing"));
    }

    #[test]
    fn error_payload_from_db_error() {
        let payload: ErrorPayload = BackendError::Db(rusqlite::Error::QueryReturnedNoRows).into();
        assert!(matches!(payload.code, ErrorCode::DbError));
        assert!(!payload.message.is_empty());
    }

    #[test]
    fn error_payload_from_internal_error() {
        let payload: ErrorPayload = BackendError::Internal("oops".to_string()).into();
        assert!(matches!(payload.code, ErrorCode::InternalError));
        assert_eq!(payload.message, "oops");
    }

    #[test]
    fn backend_error_display_messages() {
        assert_eq!(
            BackendError::Validation(crate::i18n::tr::playlist_error_name_empty()).to_string(),
            "validation error: playlist name must not be empty"
        );
        assert_eq!(
            BackendError::NotFound(crate::i18n::tr::error_track_not_found("y")).to_string(),
            "not found: track not found: y"
        );
        assert_eq!(
            BackendError::Internal("z".to_string()).to_string(),
            "internal error: z"
        );
    }
}
