//! [`AuthError`] and its mapping onto [`lakehouse_core::ApiError`].
//!
//! # Non-enumeration is load-bearing
//!
//! [`AuthError::InvalidCredentials`] is deliberately the single variant for
//! both "no such email" and "email exists, wrong password" — see
//! [`crate::password`] for how the lookup itself is written to keep those
//! two cases indistinguishable by timing as well as by message. Adding a
//! second variant (e.g. `UnknownIdentifier`) to disambiguate them in a
//! future change would reopen a user-enumeration oracle; if that
//! distinction is ever needed it belongs in a server-side `tracing` log,
//! never in this enum or in [`ApiError`].
//!
//! # No `sqlx::Error` text ever reaches a response
//!
//! [`AuthError::Database`] holds the underlying `sqlx::Error` as a
//! `#[source]` (for server-side logs/`tracing`) but its `Display` is the
//! fixed string `"database error"` — never `"{0}"` — for the same reason
//! `lakehouse_store::StoreError::Database` does this: a raw `sqlx::Error`
//! can embed the connection string, host, or driver-internal detail, and
//! that must never reach an HTTP response body.

use lakehouse_core::ApiError;
use thiserror::Error;

/// Errors this crate's authenticators, session/token repository functions,
/// and password hashing can produce.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The presented password credential is wrong, OR no such identity
    /// exists. Intentionally one variant — see the module doc comment.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// The presented session token doesn't resolve to a live, unrevoked,
    /// unexpired session.
    #[error("session is invalid or expired")]
    SessionInvalid,
    /// The presented service token doesn't resolve to a live, unrevoked,
    /// unexpired service credential.
    #[error("service credential is invalid or expired")]
    ServiceCredentialInvalid,
    /// This [`crate::Authenticator`] was handed a [`crate::Credential`]
    /// variant it does not implement (e.g. a [`crate::Credential::Bearer`]
    /// given to [`crate::password::LocalPasswordAuthenticator`]). A caller
    /// wiring authenticators together (Task 3.2) should treat this as "try
    /// the next authenticator", not as a failed login.
    #[error("credential type not supported by this authenticator")]
    UnsupportedCredential,
    /// The account this operation targets already has the identity/session
    /// being created (e.g. linking a second `local` identity to a user
    /// that already has one).
    #[error("{0}")]
    Conflict(String),
    /// The account this operation targets does not exist.
    #[error("record not found")]
    NotFound,
    /// Any other database failure. Never renders the source error — see
    /// the module doc comment.
    #[error("database error")]
    Database(#[source] sqlx::Error),
    /// Password hashing or verification failed for a reason that is not
    /// "the password was wrong" (e.g. the stored hash is not valid
    /// `Argon2` `PHC` string — a data-corruption bug, not a caller error).
    #[error("password hashing failed")]
    Hash,
}

impl From<sqlx::Error> for AuthError {
    fn from(err: sqlx::Error) -> Self {
        if matches!(err, sqlx::Error::RowNotFound) {
            return Self::NotFound;
        }
        if let sqlx::Error::Database(db_err) = &err {
            if db_err.is_unique_violation() {
                return Self::Conflict("a record with that value already exists".to_owned());
            }
        }
        Self::Database(err)
    }
}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::InvalidCredentials => Self::invalid_or_expired(),
            AuthError::SessionInvalid | AuthError::ServiceCredentialInvalid => {
                Self::invalid_or_expired()
            }
            AuthError::UnsupportedCredential => Self::unauthorized(),
            AuthError::Conflict(message) => Self::Conflict(message),
            AuthError::NotFound => Self::NotFound("record not found".to_owned()),
            AuthError::Database(_) | AuthError::Hash => {
                Self::Internal("authentication error".to_owned())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn invalid_credentials_maps_to_401_invalid_or_expired() {
        let api: ApiError = AuthError::InvalidCredentials.into();
        assert_eq!(api.status(), 401);
        assert_eq!(api.to_string(), "invalid_or_expired");
    }

    #[test]
    fn session_invalid_and_service_credential_invalid_are_indistinguishable_to_the_caller() {
        let session: ApiError = AuthError::SessionInvalid.into();
        let service: ApiError = AuthError::ServiceCredentialInvalid.into();
        assert_eq!(session.to_string(), service.to_string());
        assert_eq!(session.status(), service.status());
    }

    #[test]
    fn conflict_maps_to_409() {
        let api: ApiError = AuthError::Conflict("already linked".to_owned()).into();
        assert_eq!(api.status(), 409);
    }

    #[test]
    fn not_found_maps_to_404() {
        let api: ApiError = AuthError::NotFound.into();
        assert_eq!(api.status(), 404);
    }

    /// The regression test for the module's central promise: a database
    /// error carrying a connection string or driver detail must never leak
    /// into the rendered message.
    #[test]
    fn database_error_display_never_contains_the_source_error_text() {
        let err = AuthError::Database(sqlx::Error::Protocol(
            "postgres://lakehouse:supersecretpassword@10.0.0.5:5432/lakehouse".to_owned(),
        ));
        assert_eq!(err.to_string(), "database error");
        assert!(!err.to_string().contains("supersecretpassword"));
        let api: ApiError = err.into();
        assert_eq!(api.to_string(), "authentication error");
        assert!(!api.to_string().contains("supersecretpassword"));
    }

    #[test]
    fn unsupported_credential_maps_to_401() {
        let api: ApiError = AuthError::UnsupportedCredential.into();
        assert_eq!(api.status(), 401);
    }
}
