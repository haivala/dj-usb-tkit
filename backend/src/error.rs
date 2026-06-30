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
    InternalError,
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
                    code: ErrorCode::InternalError,
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
