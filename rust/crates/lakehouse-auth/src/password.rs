//! Local password authentication: hashing, verification, and the
//! `provider = 'local'` rows in `auth_identity`.
//!
//! # Non-enumeration
//!
//! [`verify`] never lets a caller distinguish "no such email" from "wrong
//! password" — both paths return [`AuthError::InvalidCredentials`] and,
//! critically, both paths run a full `Argon2id` verification: an unknown
//! email verifies against [`DUMMY_HASH`] (a fixed, real `Argon2id` hash of
//! an arbitrary constant) instead of returning early. Returning early on
//! "no such user" would make the response latency itself an oracle — a
//! network observer could time login attempts and learn which email
//! addresses have accounts purely from the elapsed time of the `Argon2`
//! step being skipped. Always doing the hash makes both branches do
//! comparable work.
//!
//! # `must_change_password`
//!
//! `auth_identity.must_change_password` exists so a bootstrapped account
//! (an initial admin created outside any signup flow) cannot stay on its
//! bootstrap credential forever. [`create_local_identity`] takes the flag
//! explicitly at creation time; [`must_change_password`] lets a caller
//! check it after a successful [`verify`]; [`change_password`] always
//! clears it. Nothing in [`verify`] itself refuses a login because the flag
//! is set — that policy decision (allow the login but force a redirect to
//! "change your password", vs. refuse the login outright) belongs to
//! whichever later task wires this into the router, not to the
//! authentication primitive.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use async_trait::async_trait;
use rand::rngs::OsRng;
use uuid::Uuid;

use crate::authenticator::Authenticator;
use crate::credential::Credential;
use crate::error::AuthError;
use crate::principal::Principal;
use crate::repository::{self, PgPool};
use crate::secret::Secret;

/// A real, valid `Argon2id` `PHC` hash of a fixed, non-secret constant.
/// Used only as the comparison target when no `app_user` matches the
/// presented email, so that branch pays the same `Argon2` cost a real
/// verification would — see the module doc comment. This value carries no
/// security weight of its own (the password it hashes is public, right
/// here in the source); its only job is to be a well-formed hash the
/// `Argon2` verifier will spend real time computing against.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$ZHVtbXlkdW1teWR1bW15ZHU$ULriu52Oq084VHtqRsKmm3fk5d5irs4oEOfP2zcDMsY";

/// Hash `password` with `Argon2id` at the crate's chosen parameters
/// ([`Argon2::default`], which is the OWASP-recommended minimum: 19 MiB
/// memory, 2 iterations, 1 degree of parallelism).
///
/// # Errors
///
/// Returns [`AuthError::Hash`] if hashing fails (in practice this only
/// happens for pathological inputs `Argon2` itself rejects, e.g. an
/// extremely long password).
pub fn hash_password(password: &Secret) -> Result<Secret, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.expose().as_bytes(), &salt)
        .map_err(|_| AuthError::Hash)?;
    Ok(Secret::new(hash.to_string()))
}

/// Verify `password` against a stored `Argon2id` `PHC` hash. Constant-time
/// by construction — `Argon2`'s own verifier does the comparison, and
/// [`verify`] (the caller that matters for non-enumeration) always calls
/// this exactly once per attempt regardless of whether the identity was
/// found, per the module doc comment.
fn verify_password(password: &Secret, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.expose().as_bytes(), &parsed)
        .is_ok()
}

/// Create a `provider = 'local'` identity for `app_user_id`, storing a
/// hash of `password` (never the plaintext). `external_subject` is the
/// user's own id, as text — a local login has no external subject of its
/// own (see `0019_auth.sql`'s doc comment).
///
/// # Errors
///
/// Returns [`AuthError::Conflict`] if `app_user_id` already has a `local`
/// identity, [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn create_local_identity(
    pool: &PgPool,
    app_user_id: Uuid,
    password: &Secret,
    must_change_password: bool,
) -> Result<(), AuthError> {
    let hash = hash_password(password)?;
    sqlx::query(
        "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash, must_change_password) \
         VALUES ('local', $1, $2, $3, $4)",
    )
    .bind(app_user_id.to_string())
    .bind(app_user_id)
    .bind(hash.expose())
    .bind(must_change_password)
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace the stored password hash for `app_user_id`'s `local` identity
/// and clear `must_change_password`.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `app_user_id` has no `local`
/// identity, [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn change_password(
    pool: &PgPool,
    app_user_id: Uuid,
    new_password: &Secret,
) -> Result<(), AuthError> {
    let hash = hash_password(new_password)?;
    let affected = sqlx::query(
        "UPDATE auth_identity SET password_hash = $1, must_change_password = false, \
         updated_at = now() WHERE app_user_id = $2 AND provider = 'local'",
    )
    .bind(hash.expose())
    .bind(app_user_id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AuthError::NotFound);
    }
    Ok(())
}

/// Whether `app_user_id`'s `local` identity is still flagged
/// `must_change_password`.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `app_user_id` has no `local`
/// identity, or [`AuthError::Database`] on any other failure.
pub async fn must_change_password(pool: &PgPool, app_user_id: Uuid) -> Result<bool, AuthError> {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT must_change_password FROM auth_identity \
         WHERE app_user_id = $1 AND provider = 'local'",
    )
    .bind(app_user_id)
    .fetch_optional(pool)
    .await?;
    row.map(|(flag,)| flag).ok_or(AuthError::NotFound)
}

/// Verify an `identifier`/`password` pair and, on success, load the
/// caller's [`Principal`]. See the module doc comment for the
/// non-enumeration guarantee this makes.
///
/// # Errors
///
/// Returns [`AuthError::InvalidCredentials`] if `identifier` names no user,
/// the user has no `local` identity, or `password` does not match — these
/// three cases are indistinguishable to the caller by design. Returns
/// [`AuthError::Database`] on any other failure.
pub async fn verify(
    pool: &PgPool,
    identifier: &str,
    password: &Secret,
) -> Result<Principal, AuthError> {
    let row: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT ai.app_user_id, ai.password_hash, ai.must_change_password FROM auth_identity ai \
         JOIN app_user u ON u.id = ai.app_user_id \
         WHERE u.email = $1 AND ai.provider = 'local'",
    )
    .bind(identifier)
    .fetch_optional(pool)
    .await?;

    let Some((app_user_id, stored_hash, must_change_password)) = row else {
        // No such identity: verify against the dummy hash anyway, so this
        // branch costs the same as a real verification, then fail exactly
        // as a wrong password would.
        let _ = verify_password(password, DUMMY_HASH);
        return Err(AuthError::InvalidCredentials);
    };

    if !verify_password(password, &stored_hash) {
        return Err(AuthError::InvalidCredentials);
    }

    repository::load_principal_for_user(pool, app_user_id, "local".to_owned(), must_change_password)
        .await
        .map_err(|_| AuthError::InvalidCredentials)
}

/// [`Authenticator`] for [`Credential::Password`].
pub struct LocalPasswordAuthenticator {
    pool: PgPool,
}

impl LocalPasswordAuthenticator {
    /// Build an authenticator backed by `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Authenticator for LocalPasswordAuthenticator {
    // The trait returns `&str` tied to `&self`, not `&'static str`, because a
    // future `OidcAuthenticator` needs to return an owned, per-instance
    // provider label (e.g. "oidc:okta" built from config) -- see
    // `Authenticator`'s doc comment. This impl happens to return a literal.
    #[allow(clippy::unnecessary_literal_bound)]
    fn provider_id(&self) -> &str {
        "local"
    }

    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        let Credential::Password {
            identifier,
            password,
        } = credential
        else {
            return Err(AuthError::UnsupportedCredential);
        };
        verify(&self.pool, identifier, password).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_freshly_hashed_password_verifies_against_its_own_hash() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        assert!(verify_password(&password, hash.expose()));
    }

    #[test]
    fn the_wrong_password_does_not_verify() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        assert!(!verify_password(
            &Secret::new("wrong password"),
            hash.expose()
        ));
    }

    #[test]
    fn a_malformed_stored_hash_fails_closed_rather_than_panicking() {
        assert!(!verify_password(
            &Secret::new("anything"),
            "not-a-valid-phc-hash"
        ));
    }

    #[test]
    fn hashing_the_same_password_twice_produces_different_hashes() {
        // Distinct random salts per call — this is what actually stops a
        // rainbow-table / equal-hash-implies-equal-password attack.
        let password = Secret::new("correct horse battery staple");
        let first = hash_password(&password).unwrap();
        let second = hash_password(&password).unwrap();
        assert_ne!(first.expose(), second.expose());
        assert!(verify_password(&password, first.expose()));
        assert!(verify_password(&password, second.expose()));
    }

    #[test]
    fn the_dummy_hash_constant_is_a_well_formed_argon2_phc_string() {
        // If this ever fails to parse, the non-enumeration guarantee in
        // `verify` silently degrades to "unknown user fails fast" — pin the
        // constant down explicitly rather than relying on `verify_password`
        // returning `false` for both a bad hash and a bad password to hide
        // the difference.
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
    }

    #[test]
    fn hash_never_appears_in_a_debug_render_of_the_secret_wrapping_it() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        let rendered = format!("{hash:?}");
        assert!(!rendered.contains("argon2id"));
    }
}
