use serde::Serialize;
use serde_json::Value;

use crate::i18n::{Arg, IntoArg, Text};

/// No `Display`: a catalog error has a text per language, so it's shown with
/// [`BackendError::render`], logged with [`BackendError::english`], and passed into another
/// text as itself (see [`IntoArg`]).
#[derive(Debug)]
pub enum BackendError {
    /// Something the user can act on, from the text catalog: shown in the current language,
    /// logged in English. A bug's error is `Internal`, in English.
    Validation(Text),
    ValidationWithDetails(Text, Value),
    /// Something the user asked for that isn't there; see `Validation`.
    NotFound(Text),
    Io(std::io::Error),
    Db(rusqlite::Error),
    Internal(String),
}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<rusqlite::Error> for BackendError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

/// As another text's argument: a catalog text renders with it, in its language and without the
/// kind prefix; any other error is English data, as [`BackendError::english`] has it.
impl IntoArg for &BackendError {
    fn into_arg(self) -> Arg {
        match self {
            BackendError::Validation(text)
            | BackendError::ValidationWithDetails(text, _)
            | BackendError::NotFound(text) => text.into_arg(),
            other => Arg::Text(other.english()),
        }
    }
}

impl IntoArg for BackendError {
    fn into_arg(self) -> Arg {
        (&self).into_arg()
    }
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

    /// `error`'s payload with its message inside `context`, a text saying what failed
    /// ("Rename failed: …"), in the response's language and in English for the log. The code
    /// and details stay `error`'s.
    pub fn within(error: BackendError, context: impl FnOnce(Arg) -> Text) -> Self {
        let catalog_text = match &error {
            BackendError::Validation(text)
            | BackendError::ValidationWithDetails(text, _)
            | BackendError::NotFound(text) => Some(text.into_arg()),
            _ => None,
        };
        let payload = Self::from(error);
        let text = context(catalog_text.unwrap_or_else(|| Arg::Text(payload.message.clone())));
        Self {
            message: text.render(),
            log_message: Some(text.render_english()),
            ..payload
        }
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
    /// prefix [`Self::english`] adds; any other error as [`Self::english`] has it.
    pub fn render(&self) -> String {
        match self {
            BackendError::Validation(text)
            | BackendError::ValidationWithDetails(text, _)
            | BackendError::NotFound(text) => text.render(),
            other => other.english(),
        }
    }

    /// The log file's text: English, with the error's kind first.
    pub fn english(&self) -> String {
        match self {
            BackendError::Validation(text) | BackendError::ValidationWithDetails(text, _) => {
                format!("validation error: {}", text.render_english())
            }
            BackendError::NotFound(text) => format!("not found: {}", text.render_english()),
            BackendError::Io(err) => format!("io error: {err}"),
            BackendError::Db(err) => format!("database error: {err}"),
            BackendError::Internal(msg) => format!("internal error: {msg}"),
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
    fn an_error_argument_is_its_catalog_text_or_english_data() {
        use crate::i18n::tr;
        let catalog_error = BackendError::Validation(tr::playlist_error_name_empty());
        assert_eq!(
            tr::repair_outcome_failed("Fix X", &catalog_error).render_english(),
            "Fix X failed: playlist name must not be empty"
        );
        let io_error = BackendError::Io(std::io::Error::other("disk gone"));
        assert_eq!(
            tr::repair_outcome_failed("Fix X", io_error).render(),
            "Fix X failed: io error: disk gone"
        );
    }

    #[test]
    fn an_error_within_its_context_keeps_its_code_and_details() {
        use crate::i18n::tr;
        let details = serde_json::json!({"field": "x"});
        let payload = ErrorPayload::within(
            BackendError::ValidationWithDetails(tr::playlist_error_name_empty(), details.clone()),
            tr::playlist_rename_failed,
        );
        assert!(matches!(payload.code, ErrorCode::ValidationError));
        assert_eq!(payload.details, Some(details));
        assert_eq!(
            payload.message,
            "Rename failed: playlist name must not be empty"
        );
        assert_eq!(
            payload.log_text(),
            "Rename failed: playlist name must not be empty"
        );

        let payload = ErrorPayload::within(
            BackendError::Internal("oops".to_string()),
            tr::playlist_rename_failed,
        );
        assert!(matches!(payload.code, ErrorCode::InternalError));
        assert_eq!(payload.message, "Rename failed: oops");
    }

    #[test]
    fn backend_error_english_messages() {
        assert_eq!(
            BackendError::Validation(crate::i18n::tr::playlist_error_name_empty()).english(),
            "validation error: playlist name must not be empty"
        );
        assert_eq!(
            BackendError::NotFound(crate::i18n::tr::error_track_not_found("y")).english(),
            "not found: track not found: y"
        );
        assert_eq!(
            BackendError::Internal("z".to_string()).english(),
            "internal error: z"
        );
    }
}
