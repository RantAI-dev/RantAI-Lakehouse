//! [`Credential`]: everything a caller can present as proof of identity, in
//! the exact shape [`crate::Authenticator::authenticate`] consumes.

use crate::secret::Secret;

/// Proof of identity a caller presents.
///
/// [`Self::Bearer`] is not consumed by any [`crate::Authenticator`]
/// implemented in this crate — it exists now, unused, specifically so a
/// future `OIDC` authenticator (Task 3.5) needs no change to this enum. See
/// [`crate::Authenticator`]'s doc comment for how it plugs in.
pub enum Credential {
    /// A local username/password pair. `identifier` is the `app_user.email`
    /// to look up; `password` is the plaintext the caller typed, wrapped so
    /// it cannot be logged by accident on the way to
    /// [`crate::password::LocalPasswordAuthenticator`].
    Password {
        /// The email address identifying the account.
        identifier: String,
        /// The plaintext password.
        password: Secret,
    },
    /// An opaque session token, as issued by [`crate::session::create_session`]
    /// and sent back by the browser on every request thereafter.
    SessionToken(Secret),
    /// An opaque service credential, as looked up in `service_credential`.
    ServiceToken(Secret),
    /// A bearer token intended for a signature/claims-based authenticator
    /// (a `JWT` id token from an `OIDC` provider). Opaque to this crate:
    /// nothing here parses or validates it. See the module doc comment.
    Bearer(Secret),
}

impl std::fmt::Debug for Credential {
    /// Redacts every secret payload; only the discriminant (and, for
    /// [`Self::Password`], the non-secret `identifier`) is rendered.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { identifier, .. } => f
                .debug_struct("Password")
                .field("identifier", identifier)
                .field("password", &"Secret(REDACTED)")
                .finish(),
            Self::SessionToken(_) => write!(f, "SessionToken(Secret(REDACTED))"),
            Self::ServiceToken(_) => write!(f, "ServiceToken(Secret(REDACTED))"),
            Self::Bearer(_) => write!(f, "Bearer(Secret(REDACTED))"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn password_debug_redacts_the_password_but_keeps_the_identifier() {
        let credential = Credential::Password {
            identifier: "rina@meridian.example".to_owned(),
            password: Secret::new("hunter2-do-not-leak"),
        };
        let rendered = format!("{credential:?}");
        assert!(rendered.contains("rina@meridian.example"));
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn session_token_debug_redacts_the_token() {
        let credential = Credential::SessionToken(Secret::new("super-secret-session-token"));
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("super-secret-session-token"));
    }

    #[test]
    fn service_token_debug_redacts_the_token() {
        let credential = Credential::ServiceToken(Secret::new("super-secret-service-token"));
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("super-secret-service-token"));
    }

    #[test]
    fn bearer_debug_redacts_the_token() {
        let credential = Credential::Bearer(Secret::new("super-secret-jwt-value"));
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("super-secret-jwt-value"));
    }
}
