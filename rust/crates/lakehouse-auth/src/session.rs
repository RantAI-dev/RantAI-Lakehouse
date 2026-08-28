//! Opaque, revocable browser sessions.
//!
//! # Why opaque, not a `JWT`
//!
//! The browser path uses a random opaque token rather than a
//! self-contained `JWT` specifically because it must be revocable: sign a
//! user out, or force every session dead on a password change, and the
//! very next request with that token is rejected — a lookup against
//! `session.revoked_at` sees the change immediately. A `JWT` is valid
//! until its own `exp` regardless of what the issuing server wants
//! afterward (short of maintaining a revocation list anyway, which is just
//! this table with extra steps). [`crate::credential::Credential`] still
//! carries a separate [`crate::credential::Credential::Bearer`] variant for
//! a signature-validated token, so a future `OIDC` id token is handled on
//! its own terms without redesigning this module — see
//! [`crate::Authenticator`]'s doc comment.
//!
//! # Storage
//!
//! The token handed to the browser is 32 CSPRNG bytes, hex-encoded. Only
//! its SHA-256 digest (`session.token_hash`) is ever written to Postgres —
//! see `0019_auth.sql`'s doc comment for why. A lookup by that digest is
//! not a manual byte-by-byte comparison in this crate's own code (which is
//! where a timing side-channel would have to live); it is a Postgres
//! index equality lookup, which requires already possessing the digest —
//! there is nothing for a timing difference in *finding* the row to leak
//! about the secret itself, unlike comparing a caller-supplied guess
//! byte-by-byte against a known value.

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::authenticator::Authenticator;
use crate::credential::Credential;
use crate::error::AuthError;
use crate::principal::Principal;
use crate::repository::{self, PgPool};
use crate::secret::Secret;
use crate::token::{generate_opaque_token, hash_token};

/// How long a freshly created session is valid for, absent a caller
/// override.
pub const DEFAULT_SESSION_TTL: Duration = Duration::hours(24);

/// Create a session for `app_user_id`, valid for `ttl`.
///
/// Returns the raw token — this is the ONLY point in its lifetime the raw
/// value exists outside the caller's hands; only its hash is persisted.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on any storage failure. A token-hash
/// collision would surface as [`AuthError::Conflict`], but is not expected
/// in practice (see [`generate_opaque_token`]'s doc comment).
pub async fn create_session(
    pool: &PgPool,
    app_user_id: Uuid,
    ttl: Duration,
    created_ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<Secret, AuthError> {
    let token = generate_opaque_token();
    let token_hash = hash_token(&token);
    let expires_at = OffsetDateTime::now_utc() + ttl;
    sqlx::query(
        "INSERT INTO session (app_user_id, token_hash, expires_at, created_ip, user_agent) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(app_user_id)
    .bind(token_hash)
    .bind(expires_at)
    .bind(created_ip)
    .bind(user_agent)
    .execute(pool)
    .await?;
    Ok(token)
}

/// Validate `token`, returning the [`Principal`] for its owner if the
/// session is live (not revoked, not expired).
///
/// # Errors
///
/// Returns [`AuthError::SessionInvalid`] if `token` doesn't hash to a
/// known, unrevoked, unexpired session, or [`AuthError::Database`] on any
/// other failure.
pub async fn validate_session(pool: &PgPool, token: &Secret) -> Result<Principal, AuthError> {
    let token_hash = hash_token(token);
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT app_user_id FROM session \
         WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    let Some((app_user_id,)) = row else {
        return Err(AuthError::SessionInvalid);
    };
    repository::load_principal_for_user(pool, app_user_id, "session".to_owned())
        .await
        .map_err(|_| AuthError::SessionInvalid)
}

/// Revoke `token` (sign-out). Idempotent: revoking an already-revoked,
/// expired, or unknown token is not an error — a caller signing out never
/// needs to distinguish "your session was already gone" from "you're now
/// signed out"; both leave the caller with no valid session, which is the
/// only outcome that matters.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn revoke_session(pool: &PgPool, token: &Secret) -> Result<(), AuthError> {
    let token_hash = hash_token(token);
    sqlx::query(
        "UPDATE session SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke every live session belonging to `app_user_id`. Used after a
/// password change, so a credential compromise doesn't leave old sessions
/// valid.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn revoke_all_sessions_for_user(
    pool: &PgPool,
    app_user_id: Uuid,
) -> Result<(), AuthError> {
    sqlx::query(
        "UPDATE session SET revoked_at = now() \
         WHERE app_user_id = $1 AND revoked_at IS NULL",
    )
    .bind(app_user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// [`Authenticator`] for [`Credential::SessionToken`].
pub struct SessionAuthenticator {
    pool: PgPool,
}

impl SessionAuthenticator {
    /// Build an authenticator backed by `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Authenticator for SessionAuthenticator {
    // The trait returns `&str` tied to `&self`, not `&'static str`, because a
    // future `OidcAuthenticator` needs to return an owned, per-instance
    // provider label (e.g. "oidc:okta" built from config) -- see
    // `Authenticator`'s doc comment. This impl happens to return a literal.
    #[allow(clippy::unnecessary_literal_bound)]
    fn provider_id(&self) -> &str {
        "session"
    }

    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        let Credential::SessionToken(token) = credential else {
            return Err(AuthError::UnsupportedCredential);
        };
        validate_session(&self.pool, token).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn default_session_ttl_is_24_hours() {
        assert_eq!(DEFAULT_SESSION_TTL, Duration::hours(24));
    }
}
