use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("validation error: {0}")]
    ValidationWithDetails(String, Value),
    #[error("not found: {0}")]
    NotFound(String),
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
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl From<BackendError> for ErrorPayload {
    fn from(value: BackendError) -> Self {
        match value {
            BackendError::Validation(msg) => Self {
                code: ErrorCode::ValidationError,
                message: msg,
                details: None,
            },
            BackendError::ValidationWithDetails(msg, details) => Self {
                code: ErrorCode::ValidationError,
                message: msg,
                details: Some(details),
            },
            BackendError::NotFound(msg) => Self {
                code: ErrorCode::NotFound,
                message: msg,
                details: None,
            },
            BackendError::Io(err) => Self {
                code: ErrorCode::IoError,
                message: err.to_string(),
                details: None,
            },
            BackendError::Db(_err) => {
                #[cfg(debug_assertions)]
                let message = format!("database failure: {_err}");
                #[cfg(not(debug_assertions))]
                let message = "an internal database error occurred".to_string();
                Self {
                    code: ErrorCode::DbError,
                    message,
                    details: None,
                }
            }
            BackendError::Internal(msg) => Self {
                code: ErrorCode::InternalError,
                message: msg,
                details: None,
            },
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
