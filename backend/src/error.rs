use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::i18n::Text;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("validation error: {0}")]
    Validation(String),
    /// A validation error from the text catalog: shown in the current language, logged in
    /// English. Piece 11 of the text-catalog plan moves the `Validation(String)` sites here.
    #[error("validation error: {}", .0.render_english())]
    ValidationText(Text),
    #[error("validation error: {0}")]
    ValidationWithDetails(String, Value),
    #[error("not found: {0}")]
    NotFound(String),
    /// A not-found error from the text catalog; see `ValidationText`.
    #[error("not found: {}", .0.render_english())]
    NotFoundText(Text),
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
            BackendError::Validation(msg) => Self::new(ErrorCode::ValidationError, msg),
            BackendError::ValidationText(text) => {
                Self::from_text(ErrorCode::ValidationError, &text)
            }
            BackendError::ValidationWithDetails(msg, details) => Self {
                details: Some(details),
                ..Self::new(ErrorCode::ValidationError, msg)
            },
            BackendError::NotFound(msg) => Self::new(ErrorCode::NotFound, msg),
            BackendError::NotFoundText(text) => Self::from_text(ErrorCode::NotFound, &text),
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
        let payload: ErrorPayload = BackendError::Validation("bad input".to_string()).into();
        assert!(matches!(payload.code, ErrorCode::ValidationError));
        assert_eq!(payload.message, "bad input");
        assert!(payload.details.is_none());
    }

    #[test]
    fn error_payload_from_validation_with_details() {
        let details = serde_json::json!({"field": "x"});
        let payload: ErrorPayload =
            BackendError::ValidationWithDetails("bad input".to_string(), details.clone()).into();
        assert!(matches!(payload.code, ErrorCode::ValidationError));
        assert_eq!(payload.details, Some(details));
    }

    #[test]
    fn error_payload_from_not_found() {
        let payload: ErrorPayload = BackendError::NotFound("missing".to_string()).into();
        assert!(matches!(payload.code, ErrorCode::NotFound));
        assert_eq!(payload.message, "missing");
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
            BackendError::Validation("x".to_string()).to_string(),
            "validation error: x"
        );
        assert_eq!(
            BackendError::NotFound("y".to_string()).to_string(),
            "not found: y"
        );
        assert_eq!(
            BackendError::Internal("z".to_string()).to_string(),
            "internal error: z"
        );
    }
}
