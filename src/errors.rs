// SPDX-License-Identifier: LGPL-3.0-only

use steam_vent::{ConnectionError, EResult, LoginError, NetworkError};

/// How callers should react to a failure when retrying an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDisposition {
    /// Safe to retry immediately without backoff.
    ImmediateRetry,
    /// Retry is possible but should be delayed/backed off.
    BackoffRetry,
    /// Caller should prompt the user to re-authenticate.
    Reauthenticate,
    /// Retrying would not help; escalate to caller.
    Fatal,
}

/// High-level component where an error originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDomain {
    /// Error occurred during authentication or authorization.
    Authentication,
    /// Error occurred in network transport layer.
    Transport,
    /// Error occurred in application-level logic.
    Application,
    /// Error origin could not be determined.
    Unknown,
}

/// Summary describing how an upstream error should be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorInventoryEntry {
    /// The domain where the error originated.
    pub domain: ErrorDomain,
    /// How the caller should handle retrying this error.
    pub disposition: RetryDisposition,
    /// Human-readable description of the error.
    pub description: &'static str,
}

impl ErrorInventoryEntry {
    /// Create a new error inventory entry.
    ///
    /// # Arguments
    ///
    /// * `domain` - The domain where the error originated
    /// * `disposition` - How the caller should handle retrying
    /// * `description` - Human-readable description of the error
    ///
    /// # Returns
    ///
    /// A new `ErrorInventoryEntry` with the specified classification.
    pub const fn new(
        domain: ErrorDomain,
        disposition: RetryDisposition,
        description: &'static str,
    ) -> Self {
        Self {
            domain,
            disposition,
            description,
        }
    }
}

/// Classify a top-level connection error returned by steam-vent.
pub fn classify_connection_error(err: &ConnectionError) -> ErrorInventoryEntry {
    match err {
        ConnectionError::AccessToken(_) => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Reauthenticate,
            "stale or invalid access token",
        ),
        ConnectionError::Network(net) => classify_network_error(net),
        ConnectionError::LoginError(login) => classify_login_error(login),
        ConnectionError::Aborted => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::Fatal,
            "operation aborted by client",
        ),
        ConnectionError::UnsupportedConfirmationAction(_) => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Reauthenticate,
            "two-factor confirmation required",
        ),
        _ => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::BackoffRetry,
            "unclassified connection error",
        ),
    }
}

/// Classify lower-level network failures.
pub fn classify_network_error(err: &NetworkError) -> ErrorInventoryEntry {
    match err {
        NetworkError::Timeout => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::ImmediateRetry,
            "request timed out",
        ),
        NetworkError::EOF | NetworkError::IO(_) | NetworkError::Ws(_) => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::BackoffRetry,
            "transport dropped connection",
        ),
        NetworkError::DifferentMessage(_, _)
        | NetworkError::DifferentServiceMethod(_, _)
        | NetworkError::InvalidHeader
        | NetworkError::InvalidMessageKind(_) => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::Fatal,
            "protocol mismatch",
        ),
        NetworkError::ApiError(result) => classify_api_error(*result),
        NetworkError::CryptoHandshakeFailed | NetworkError::CryptoError(_) => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::BackoffRetry,
            "crypto handshake failure",
        ),
        NetworkError::MalformedBody(_) => ErrorInventoryEntry::new(
            ErrorDomain::Application,
            RetryDisposition::Fatal,
            "malformed response payload",
        ),
        _ => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::BackoffRetry,
            "unclassified network error",
        ),
    }
}

/// Classify login errors into retry strategies.
pub fn classify_login_error(err: &LoginError) -> ErrorInventoryEntry {
    match err {
        LoginError::InvalidCredentials | LoginError::InvalidSteamId => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Fatal,
            "invalid credentials",
        ),
        LoginError::SteamGuardRequired => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Reauthenticate,
            "additional confirmation required",
        ),
        LoginError::RateLimited => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::BackoffRetry,
            "rate limited by Steam",
        ),
        LoginError::UnavailableAccount => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Fatal,
            "account unavailable",
        ),
        LoginError::InvalidPubKey(_) => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::BackoffRetry,
            "invalid public key during login",
        ),
        LoginError::Unknown(_) => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::BackoffRetry,
            "unknown login failure",
        ),
        _ => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::BackoffRetry,
            "unclassified login error",
        ),
    }
}

fn classify_api_error(result: EResult) -> ErrorInventoryEntry {
    match result {
        EResult::Timeout => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::ImmediateRetry,
            "steam backend timeout",
        ),
        EResult::OK => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::Fatal,
            "unexpected OK error code",
        ),
        EResult::RateLimitExceeded
        | EResult::AccountActivityLimitExceeded
        | EResult::LimitExceeded
        | EResult::AccountLimitExceeded => ErrorInventoryEntry::new(
            ErrorDomain::Transport,
            RetryDisposition::BackoffRetry,
            "rate limited by Steam",
        ),
        EResult::InvalidPassword
        | EResult::AccountDisabled
        | EResult::AccountLockedDown
        | EResult::AccountHasBeenDeleted
        | EResult::AccountNotFound => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Fatal,
            "invalid credentials",
        ),
        EResult::AccountLoginDeniedNeedTwoFactor => ErrorInventoryEntry::new(
            ErrorDomain::Authentication,
            RetryDisposition::Reauthenticate,
            "two-factor authentication required",
        ),
        _ => ErrorInventoryEntry::new(
            ErrorDomain::Unknown,
            RetryDisposition::BackoffRetry,
            "unmapped Steam error code",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_retryable() {
        let entry =
            classify_network_error(&NetworkError::Timeout);
        assert_eq!(entry.disposition, RetryDisposition::ImmediateRetry);
    }

    #[test]
    fn invalid_credentials_are_fatal() {
        let entry = classify_login_error(&LoginError::InvalidCredentials);
        assert_eq!(entry.disposition, RetryDisposition::Fatal);
    }
}

