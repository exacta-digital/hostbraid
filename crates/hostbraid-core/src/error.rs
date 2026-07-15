use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// A HostBraid result with a stable, user-facing error.
pub type Result<T> = std::result::Result<T, AppError>;

/// Stable error categories for scripts and agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    InvalidArguments,
    InvalidInput,
    NotFound,
    AmbiguousTarget,
    Unsupported,
    Unavailable,
    AuthenticationFailed,
    ProviderUnavailable,
    DependencyMissing,
    PolicyDenied,
    Io,
    Internal,
}

impl ErrorCode {
    /// Stable process exit status associated with the error category.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::InvalidArguments | Self::InvalidInput | Self::AmbiguousTarget => 2,
            Self::DependencyMissing => 3,
            Self::AuthenticationFailed => 4,
            Self::ProviderUnavailable | Self::Unavailable => 5,
            Self::PolicyDenied => 6,
            Self::NotFound | Self::Unsupported | Self::Io | Self::Internal => 1,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::AmbiguousTarget => "ambiguous_target",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::AuthenticationFailed => "authentication_failed",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::DependencyMissing => "dependency_missing",
            Self::PolicyDenied => "policy_denied",
            Self::Io => "io",
            Self::Internal => "internal",
        };
        formatter.write_str(value)
    }
}

/// A curated public error suitable for both people and machine output.
///
/// This type intentionally has no raw source field. Callers are responsible for mapping private
/// provider or process failures to secret-free messages before constructing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{message}")]
pub struct AppError {
    code: ErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl AppError {
    /// Construct a public error from curated, secret-free text.
    ///
    /// Raw provider responses, URLs, headers, process output, and arbitrary source errors must not
    /// be passed here. Keep those in a non-serializable adapter error and map them at the boundary.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    /// Add a curated, secret-free recovery hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    #[must_use]
    pub fn io(context: &str, error: &std::io::Error) -> Self {
        Self::new(ErrorCode::Io, format!("{context} ({:?})", error.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorCode;

    #[test]
    fn error_codes_have_stable_names_and_exit_statuses() {
        assert_eq!(ErrorCode::AmbiguousTarget.to_string(), "ambiguous_target");
        assert_eq!(ErrorCode::AmbiguousTarget.exit_code(), 2);
        assert_eq!(ErrorCode::AuthenticationFailed.exit_code(), 4);
    }
}
